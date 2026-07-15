//! UI overlay du launcher, style Spotlight.
//!
//! Fenêtre sans décorations, barre de recherche + résultats. GApplication
//! garantit l'instance unique : relancer le binaire (le raccourci clavier
//! GNOME fait exactement ça) réveille l'instance résidente via D-Bus, la
//! fenêtre réapparaît instantanément. Échap la cache sans tuer le process.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;
use gtk::pango;

use weft_core::model::Icon;
use weft_core::{Action, Activation, Config, Hit, Registry, ResultItem, UninstallSpec};

/// Désinstallation demandée et en attente de confirmation (Ctrl+Suppr sur
/// la ligne sélectionnée). Tant qu'elle est là, Entrée confirme et Échap
/// annule — au lieu de lancer / fermer.
struct Pending {
    spec: UninstallSpec,
    title: String,
}

mod setup;

const APP_ID: &str = "dev.weft.Launcher";


/// Debounce du watch : on attend ce silence avant de re-scanner. Généreux
/// exprès — pendant un téléchargement Steam les manifests sont réécrits en
/// continu, il ne faut pas reconstruire l'index en boucle.
const WATCH_QUIET_SECS: u64 = 5;

/// Lancé avec --daemon (autostart de session) : construire l'UI et l'index
/// sans montrer la fenêtre. Consommé à la première activation.
static DAEMON_START: AtomicBool = AtomicBool::new(false);

struct State {
    registry: Registry,
    /// Hits actuellement affichés, même ordre que la ListBox.
    hits: Vec<Hit>,
    max_results: usize,
}

fn main() -> glib::ExitCode {
    // Mode debug CLI conservé : `weft-launcher --list [requête]`.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--list") {
        return debug_list(args.get(2).map(String::as_str).unwrap_or(""));
    }
    // Assistant à la demande.
    if args.get(1).is_some_and(|a| a == "--setup") {
        return setup::run();
    }
    if args.get(1).is_some_and(|a| a == "--daemon") {
        DAEMON_START.store(true, Ordering::SeqCst);
    }
    // Premier lancement interactif (jamais en mode daemon, qui tourne
    // sans surface) : on montre l'assistant au lieu du launcher.
    if !DAEMON_START.load(Ordering::SeqCst) && setup::is_first_run() {
        return setup::run();
    }

    let app = adw::Application::builder().application_id(APP_ID).build();

    // IMPORTANT : les handlers AVANT tout register() — g_application_register
    // émet startup immédiatement, un handler connecté après ne tourne jamais
    // (bug historique : le daemon démarrait sans thème).
    app.connect_startup(|_| {
        // Surface sombre opaque quelle que soit la préférence GNOME :
        // l'identité visuelle de Weft ne suit pas le thème du bureau.
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
        load_css();
    });
    app.connect_activate(activate);

    // Un démarrage --daemon alors qu'une instance tourne déjà ne doit PAS
    // faire surgir sa fenêtre : on sort avant toute activation.
    if DAEMON_START.load(Ordering::SeqCst) {
        let _ = app.register(gtk::gio::Cancellable::NONE);
        if app.is_remote() {
            return glib::ExitCode::SUCCESS;
        }
    }
    // GApplication consommerait argv ; on ne lui passe rien.
    app.run_with_args::<String>(&[])
}

fn activate(app: &adw::Application) {
    // Deuxième invocation : la fenêtre existe déjà (peut-être jamais
    // montrée si démarrage --daemon, donc windows() et pas active_window()),
    // on la remontre avec un index rafraîchi.
    if let Some(window) = app.windows().first() {
        refresh_and_present(window);
        return;
    }
    let window = build_ui(app);
    setup_watch(&window);
    if DAEMON_START.swap(false, Ordering::SeqCst) {
        // Autostart de session : index chaud, fenêtre cachée. Le premier
        // raccourci clavier n'aura plus qu'à présenter.
        let (state, ..) = ui_parts(window.upcast_ref());
        state.borrow_mut().registry.refresh();
    } else {
        refresh_and_present(window.upcast_ref());
    }
}

fn build_ui(app: &adw::Application) -> gtk::ApplicationWindow {
    let cfg = Config::load();
    let state = Rc::new(RefCell::new(State {
        registry: Registry::from_config(&cfg.providers),
        hits: Vec::new(),
        max_results: cfg.window.max_results,
    }));

    // Désinstallation en attente de confirmation (voir `Pending`).
    let pending: Rc<RefCell<Option<Pending>>> = Rc::new(RefCell::new(None));

    let entry = gtk::SearchEntry::builder()
        .placeholder_text("Rechercher une application…")
        .hexpand(true)
        .build();
    entry.add_css_class("weft-entry");

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("weft-list");

    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("weft-root");
    // Wayland interdit de positionner une fenêtre : la fenêtre est donc
    // maximisée et transparente, et c'est la surface qui se centre dedans
    // (l'ombre CSS se dessine librement dans la zone transparente).
    root.set_halign(gtk::Align::Center);
    root.set_valign(gtk::Align::Center);
    root.set_hexpand(true);
    root.set_vexpand(true);
    root.set_size_request(cfg.window.width, cfg.window.height);
    root.append(&entry);
    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    separator.add_css_class("weft-separator");
    root.append(&separator);
    root.append(&scroller);

    // Barre de confirmation de désinstallation, cachée tant qu'aucune n'est
    // demandée. Elle vit sous la liste et ne pousse rien quand invisible.
    let confirm = gtk::Label::new(None);
    confirm.add_css_class("weft-confirm");
    confirm.set_halign(gtk::Align::Center);
    confirm.set_wrap(true);
    confirm.set_visible(false);
    root.append(&confirm);

    // Zone transparente pleine fenêtre autour de la surface : c'est elle
    // qui reçoit les clics "à côté" (une fenêtre GTK n'est pas une cible
    // de clic elle-même, il faut un widget).
    let backdrop = gtk::Box::new(gtk::Orientation::Vertical, 0);
    backdrop.append(&root);

    // PAS de resizable(false) : mutter refuse de maximiser une fenêtre
    // non redimensionnable, et l'overlay resterait ancré en haut à gauche.
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Weft")
        .decorated(false)
        .child(&backdrop)
        .build();
    window.add_css_class("weft-window");
    window.maximize();

    // Perte de focus => fermeture (comportement Raycast/Spotlight) :
    // couvre aussi le clic hors fenêtre, où que l'événement atterrisse.
    window.connect_is_active_notify(|w| {
        if !w.is_active() && w.is_visible() {
            w.close();
        }
    });

    // Cliquer dans la zone transparente (hors surface) ferme l'overlay.
    // Gesture en phase capture sur la FENÊTRE : elle est toujours sur le
    // chemin de l'événement, quel que soit le widget cliqué ; test
    // géométrique contre les limites de la surface.
    let outside_click = gtk::GestureClick::new();
    outside_click.set_propagation_phase(gtk::PropagationPhase::Capture);
    outside_click.connect_pressed(glib::clone!(
        #[weak] window, #[weak] root,
        move |_, _, x, y| {
            let outside = root
                .compute_bounds(&window)
                .is_none_or(|b| !b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)));
            if outside {
                window.close();
            }
        }
    ));
    window.add_controller(outside_click);
    // Échap / lancement : on cache, on ne quitte pas — c'est ce qui rend la
    // réapparition instantanée.
    window.set_hide_on_close(true);

    // Frappe => re-filtrage. Toute frappe annule aussi une confirmation de
    // désinstallation en attente (l'utilisateur est passé à autre chose).
    entry.connect_search_changed(glib::clone!(
        #[strong] state, #[weak] list, #[strong] pending, #[weak] confirm,
        move |e| {
            cancel_confirm(&pending, &confirm);
            refresh_list(&state, &list, &e.text());
        }
    ));

    // Entrée => confirmer la désinstallation si une attend, sinon lancer.
    entry.connect_activate(glib::clone!(
        #[strong] state, #[weak] list, #[weak] window,
        #[strong] pending, #[weak] confirm,
        move |e| {
            if pending.borrow().is_some() {
                confirm_uninstall(&pending, &confirm, &state, e, &list);
            } else if let Some(row) = list.selected_row() {
                launch_row(&state, row.index(), &window);
            }
        }
    ));

    // Échap : annuler la confirmation si une attend, sinon fermer l'overlay.
    entry.connect_stop_search(glib::clone!(
        #[weak] window, #[strong] pending, #[weak] confirm,
        move |_| {
            if pending.borrow().is_some() {
                cancel_confirm(&pending, &confirm);
            } else {
                window.close();
            }
        }
    ));

    // Clic (ou Entrée quand une ligne a le focus). Un clic pendant une
    // confirmation l'annule au lieu de lancer.
    list.connect_row_activated(glib::clone!(
        #[strong] state, #[weak] window, #[strong] pending, #[weak] confirm,
        move |_, row| {
            if pending.borrow().is_some() {
                cancel_confirm(&pending, &confirm);
            } else {
                launch_row(&state, row.index(), &window);
            }
        }
    ));

    // Flèches haut/bas pour la sélection, Ctrl+Suppr pour désinstaller la
    // ligne sélectionnée. Phase capture : intercepter Ctrl+Suppr AVANT que
    // la barre de recherche ne l'interprète comme « effacer un mot ».
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(glib::clone!(
        #[weak] list, #[strong] state, #[strong] pending, #[weak] confirm,
        #[upgrade_or] glib::Propagation::Proceed,
        move |_, key, _, mods| {
            if key == gtk::gdk::Key::Delete
                && mods.contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                request_uninstall(&state, &list, &pending, &confirm);
                return glib::Propagation::Stop;
            }
            let delta: i32 = match key {
                gtk::gdk::Key::Down => 1,
                gtk::gdk::Key::Up => -1,
                _ => return glib::Propagation::Proceed,
            };
            let current = list.selected_row().map(|r| r.index()).unwrap_or(-1);
            if let Some(row) = list.row_at_index(current + delta) {
                list.select_row(Some(&row));
                row.grab_focus(); // fait défiler la liste jusqu'à la ligne
            }
            glib::Propagation::Stop
        }
    ));
    entry.add_controller(keys);

    // La saisie garde le clavier même si le focus visuel bouge.
    entry.set_key_capture_widget(Some(&window));

    unsafe {
        window.set_data("weft-state", state);
        window.set_data("weft-entry", entry);
        window.set_data("weft-list", list);
    }
    window
}

/// Surveille les répertoires des sources ; après WATCH_QUIET_SECS de
/// silence suivant un changement, reconstruit l'index en arrière-plan
/// (installations apt/Flatpak/Steam prises en compte sans redémarrage).
fn setup_watch(window: &gtk::ApplicationWindow) {
    use notify::{RecursiveMode, Watcher};

    let dirty = Arc::new(AtomicBool::new(false));
    let last_event = Arc::new(Mutex::new(Instant::now()));

    let (d, l) = (dirty.clone(), last_event.clone());
    // Le callback tourne sur le thread de notify : il ne touche à rien de
    // GTK, il pose juste un drapeau que le timer GTK viendra lire.
    let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            d.store(true, Ordering::SeqCst);
            *l.lock().unwrap() = Instant::now();
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("weft: watch des sources indisponible : {e}");
            return;
        }
    };

    let (state, ..) = ui_parts(window.upcast_ref());
    for spec in state.borrow().registry.watch_specs() {
        if !spec.path.is_dir() {
            continue;
        }
        let mode = if spec.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        if let Err(e) = watcher.watch(&spec.path, mode) {
            eprintln!("weft: watch impossible sur {} : {e}", spec.path.display());
        }
    }

    glib::timeout_add_seconds_local(
        2,
        glib::clone!(
            #[weak] window,
            #[upgrade_or] glib::ControlFlow::Break,
            move || {
                let quiet = last_event.lock().unwrap().elapsed()
                    >= Duration::from_secs(WATCH_QUIET_SECS);
                if dirty.load(Ordering::SeqCst) && quiet {
                    dirty.store(false, Ordering::SeqCst);
                    let (state, entry, list) = ui_parts(window.upcast_ref());
                    state.borrow_mut().registry.refresh();
                    refresh_list(&state, &list, &entry.text());
                }
                glib::ControlFlow::Continue
            }
        ),
    );

    // Le watcher s'arrête quand on le droppe : on l'attache à la fenêtre.
    unsafe {
        window.set_data("weft-watcher", watcher);
    }
}

/// Re-scanne le système, vide la recherche, montre la fenêtre.
fn refresh_and_present(window: &gtk::Window) {
    let (state, entry, list) = ui_parts(window);
    state.borrow_mut().registry.refresh();
    entry.set_text("");
    refresh_list(&state, &list, "");
    window.present();
    entry.grab_focus();
}

fn refresh_list(state: &Rc<RefCell<State>>, list: &gtk::ListBox, query: &str) {
    let mut s = state.borrow_mut();
    let State { registry, hits, max_results } = &mut *s;
    *hits = registry.query(query);
    // Requête vide : liste complète scrollable. Sinon, top résultats.
    if !query.is_empty() {
        hits.truncate(*max_results);
    }

    list.remove_all();
    for hit in hits.iter() {
        list.append(&make_row(&hit.item));
    }
    drop(s);
    // Sélection par défaut : premier résultat, prêt pour Entrée.
    list.select_row(list.row_at_index(0).as_ref());
}

// --------------------------------------------------------------------- //
// Désinstallation depuis l'overlay                                       //
// --------------------------------------------------------------------- //

/// Ctrl+Suppr sur la ligne sélectionnée : arme une confirmation si l'app
/// est désinstallable proprement, sinon le dit sans rien mettre en attente.
fn request_uninstall(
    state: &Rc<RefCell<State>>,
    list: &gtk::ListBox,
    pending: &Rc<RefCell<Option<Pending>>>,
    confirm: &gtk::Label,
) {
    let Some(row) = list.selected_row() else { return };
    let item = state
        .borrow()
        .hits
        .get(row.index() as usize)
        .map(|h| h.item.clone());
    let Some(item) = item else { return };

    match item.uninstall {
        Some(spec) => {
            confirm.set_text(&format!(
                "Désinstaller « {} » ?   ↵ confirmer    Échap annuler",
                item.title
            ));
            confirm.set_visible(true);
            *pending.borrow_mut() = Some(Pending { spec, title: item.title });
        }
        None => {
            confirm.set_text("Cette app ne se désinstalle pas depuis Weft.");
            confirm.set_visible(true);
        }
    }
}

/// Annule une confirmation en attente et cache la barre.
fn cancel_confirm(pending: &Rc<RefCell<Option<Pending>>>, confirm: &gtk::Label) {
    pending.borrow_mut().take();
    confirm.set_visible(false);
    confirm.set_text("");
}

/// Confirmée : la désinstallation tourne dans un thread (Flatpak/Weft
/// peuvent prendre une seconde), l'UI reste vivante, et la liste est
/// rafraîchie dès que c'est fait — l'app disparaît sous les yeux.
fn confirm_uninstall(
    pending: &Rc<RefCell<Option<Pending>>>,
    confirm: &gtk::Label,
    state: &Rc<RefCell<State>>,
    entry: &gtk::SearchEntry,
    list: &gtk::ListBox,
) {
    let Some(p) = pending.borrow_mut().take() else { return };
    confirm.set_text(&format!("Désinstallation de « {} »…", p.title));
    confirm.set_visible(true);

    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let spec = p.spec;
    std::thread::spawn(move || {
        let _ = tx.send(weft_core::launch::uninstall(&spec).map_err(|e| e.to_string()));
    });

    let rx = Rc::new(RefCell::new(rx));
    glib::timeout_add_local(
        Duration::from_millis(100),
        glib::clone!(
            #[weak] confirm, #[strong] state, #[weak] entry, #[weak] list,
            #[upgrade_or] glib::ControlFlow::Break,
            move || match rx.borrow().try_recv() {
                Ok(Ok(())) => {
                    confirm.set_visible(false);
                    confirm.set_text("");
                    state.borrow_mut().registry.refresh();
                    refresh_list(&state, &list, &entry.text());
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    confirm.set_text(&format!("Échec : {e}"));
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        ),
    );
}

/// Côté du gabarit d'icône : toutes les icônes occupent exactement cette
/// empreinte, la colonne de gauche reste parfaitement alignée.
const ICON_SIZE: i32 = 32;

fn make_row(item: &ResultItem) -> gtk::ListBoxRow {
    let icon = row_icon(item);
    icon.set_pixel_size(ICON_SIZE);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    // Gabarit strict : l'image vit dans une case de taille fixe.
    let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    slot.set_size_request(ICON_SIZE, ICON_SIZE);
    slot.set_valign(gtk::Align::Center);
    slot.append(&icon);
    let icon = slot;

    let name = gtk::Label::builder()
        .label(&item.title)
        .halign(gtk::Align::Start)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    name.add_css_class("weft-name");

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    text.append(&name);
    if let Some(desc) = &item.subtitle {
        let desc = gtk::Label::builder()
            .label(desc)
            .halign(gtk::Align::Start)
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        desc.add_css_class("dim-label");
        desc.add_css_class("weft-desc");
        text.append(&desc);
    }

    let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row_box.append(&icon);
    row_box.append(&text);

    // Actions clavier, visibles uniquement sur la ligne sélectionnée
    // (row:selected .weft-hint dans le CSS — aucun état côté Rust).
    let hints = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    hints.set_halign(gtk::Align::End);
    hints.set_hexpand(true);

    let verb = match &item.action {
        Action::CopyText(_) => "copier",
        Action::OpenPath(_) => "ouvrir",
        Action::Launch(_) => "lancer",
    };
    let hint = gtk::Label::new(Some(&format!("↵ {verb}")));
    hint.add_css_class("weft-hint");
    hints.append(&hint);

    // Indice de désinstallation : seulement là où c'est sûr (apps Weft,
    // Flatpak, Steam) — jamais sur une native apt.
    if item.uninstall.is_some() {
        let uninstall_hint = gtk::Label::new(Some("⌦ désinstaller"));
        uninstall_hint.add_css_class("weft-hint");
        hints.append(&uninstall_hint);
    }

    row_box.append(&hints);

    let row = gtk::ListBoxRow::builder().child(&row_box).build();
    row.add_css_class("weft-row");
    row
}

fn row_icon(item: &ResultItem) -> gtk::Image {
    match &item.icon {
        Some(Icon::Named(name)) => {
            // Nom absent du thème : GTK afficherait une image cassée, on
            // préfère un fallback discret.
            let in_theme = gtk::gdk::Display::default()
                .is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(name));
            if in_theme {
                gtk::Image::from_icon_name(name)
            } else {
                fallback_icon()
            }
        }
        Some(Icon::Path(path)) => gtk::Image::from_file(path),
        // Fichier sans icône imposée : icône du type MIME, déduite du nom
        // (pas de lecture du contenu — la liste doit rester instantanée).
        None => {
            if let Action::OpenPath(path) = &item.action {
                let (ctype, _) = gtk::gio::functions::content_type_guess(Some(path), &[]);
                gtk::Image::from_gicon(&gtk::gio::functions::content_type_get_icon(&ctype))
            } else {
                fallback_icon()
            }
        }
    }
}

/// Icône de secours : symbolique, atténuée par le CSS — assume son statut
/// de fallback au lieu d'imiter une vraie icône.
fn fallback_icon() -> gtk::Image {
    let img = gtk::Image::from_icon_name("application-x-executable-symbolic");
    img.add_css_class("weft-icon-dim");
    img
}

fn launch_row(state: &Rc<RefCell<State>>, row_index: i32, window: &gtk::ApplicationWindow) {
    let mut s = state.borrow_mut();
    let State { registry, hits, .. } = &mut *s;
    let Some(hit) = hits.get(row_index as usize) else { return };
    match registry.activate(hit) {
        Ok(Activation::Done) => {
            drop(s);
            window.close(); // hide_on_close => juste caché
        }
        Ok(Activation::CopyRequested(text)) => {
            window.clipboard().set_text(&text);
            drop(s);
            window.close();
        }
        Err(e) => eprintln!("weft: échec de « {} » : {e}", hit.item.title),
    }
}

fn ui_parts(window: &gtk::Window) -> (Rc<RefCell<State>>, gtk::SearchEntry, gtk::ListBox) {
    unsafe {
        (
            window.data::<Rc<RefCell<State>>>("weft-state").unwrap().as_ref().clone(),
            window.data::<gtk::SearchEntry>("weft-entry").unwrap().as_ref().clone(),
            window.data::<gtk::ListBox>("weft-list").unwrap().as_ref().clone(),
        )
    }
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    // Tous les tokens visuels vivent dans theme.css, embarqué au build.
    provider.load_from_string(include_str!("theme.css"));
    match gtk::gdk::Display::default() {
        Some(display) => {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            eprintln!("weft: thème chargé (display {})", display.name());

            // Override utilisateur : ~/.config/weft/theme.css, appliqué
            // par-dessus le thème embarqué. Permet d'itérer sur les tokens
            // (accent, fond...) avec un simple restart, sans recompiler.
            if let Some(path) = weft_core::config::config_path()
                .map(|p| p.with_file_name("theme.css"))
                .filter(|p| p.is_file())
            {
                let user = gtk::CssProvider::new();
                user.load_from_path(&path);
                gtk::style_context_add_provider_for_display(
                    &display,
                    &user,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
                );
                eprintln!("weft: override utilisateur chargé ({})", path.display());
            }
        }
        // Sans display au démarrage (daemon lancé très tôt dans la
        // session), le thème ne serait jamais appliqué : trace claire.
        None => eprintln!("weft: PAS de display au chargement du thème !"),
    }
}

fn debug_list(query: &str) -> glib::ExitCode {
    let start = std::time::Instant::now();
    let mut registry = Registry::from_config(&Config::load().providers);
    let scan_time = start.elapsed();
    let hits = registry.query(query);
    for hit in hits.iter().take(15) {
        let uninstall = match &hit.item.uninstall {
            Some(UninstallSpec::WeftWindows(_)) => "  ⌦ weft",
            Some(UninstallSpec::Flatpak(_)) => "  ⌦ flatpak",
            Some(UninstallSpec::Steam(_)) => "  ⌦ steam",
            None => "",
        };
        println!(
            "{:<40} [{:?}, {}]{uninstall}",
            hit.item.title, hit.item.tier, hit.item.score
        );
    }
    eprintln!("\nscan en {scan_time:?}, {} résultats", hits.len());
    glib::ExitCode::SUCCESS
}
