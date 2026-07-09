//! UI overlay du launcher, style Spotlight.
//!
//! Fenêtre sans décorations, barre de recherche + résultats. GApplication
//! garantit l'instance unique : relancer le binaire (le raccourci clavier
//! GNOME fait exactement ça) réveille l'instance résidente via D-Bus, la
//! fenêtre réapparaît instantanément. Échap la cache sans tuer le process.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;
use gtk::pango;

use weft_core::model::Icon;
use weft_core::{Action, Activation, Config, Hit, Registry, ResultItem};

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
    if args.get(1).is_some_and(|a| a == "--daemon") {
        DAEMON_START.store(true, Ordering::SeqCst);
    }

    let app = adw::Application::builder().application_id(APP_ID).build();

    // Un démarrage --daemon alors qu'une instance tourne déjà ne doit PAS
    // faire surgir sa fenêtre : on sort avant toute activation.
    if DAEMON_START.load(Ordering::SeqCst) {
        let _ = app.register(gtk::gio::Cancellable::NONE);
        if app.is_remote() {
            return glib::ExitCode::SUCCESS;
        }
    }

    app.connect_startup(|_| {
        // Surface sombre opaque quelle que soit la préférence GNOME :
        // l'identité visuelle de Weft ne suit pas le thème du bureau.
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
        load_css();
    });
    app.connect_activate(activate);
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
    root.set_size_request(cfg.window.width, cfg.window.height);
    root.append(&entry);
    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    separator.add_css_class("weft-separator");
    root.append(&separator);
    root.append(&scroller);

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Weft")
        .resizable(false)
        .decorated(false)
        .child(&root)
        .build();
    window.add_css_class("weft-window");
    window.maximize();

    // Cliquer dans la zone transparente (hors surface) ferme l'overlay.
    let outside_click = gtk::GestureClick::new();
    outside_click.set_propagation_phase(gtk::PropagationPhase::Target);
    outside_click.connect_pressed(glib::clone!(
        #[weak] window,
        move |_, _, _, _| window.close()
    ));
    window.add_controller(outside_click);
    // Échap / lancement : on cache, on ne quitte pas — c'est ce qui rend la
    // réapparition instantanée.
    window.set_hide_on_close(true);

    // Frappe => re-filtrage.
    entry.connect_search_changed(glib::clone!(
        #[strong] state, #[weak] list,
        move |e| refresh_list(&state, &list, &e.text())
    ));

    // Entrée => lancer la sélection.
    entry.connect_activate(glib::clone!(
        #[strong] state, #[weak] list, #[weak] window,
        move |_| {
            if let Some(row) = list.selected_row() {
                launch_row(&state, row.index(), &window);
            }
        }
    ));

    // Échap dans la barre de recherche.
    entry.connect_stop_search(glib::clone!(
        #[weak] window,
        move |_| { window.close(); }
    ));

    // Clic (ou Entrée quand une ligne a le focus).
    list.connect_row_activated(glib::clone!(
        #[strong] state, #[weak] window,
        move |_, row| launch_row(&state, row.index(), &window)
    ));

    // Flèches haut/bas depuis la barre de recherche : déplacer la sélection
    // sans perdre le focus clavier de la saisie.
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak] list,
        #[upgrade_or] glib::Propagation::Proceed,
        move |_, key, _, _| {
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

fn make_row(item: &ResultItem) -> gtk::ListBoxRow {
    let icon = row_icon(item);
    icon.set_pixel_size(32);

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
                gtk::Image::from_icon_name("application-x-executable-symbolic")
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
                gtk::Image::from_icon_name("application-x-executable-symbolic")
            }
        }
    }
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
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn debug_list(query: &str) -> glib::ExitCode {
    let start = std::time::Instant::now();
    let mut registry = Registry::from_config(&Config::load().providers);
    let scan_time = start.elapsed();
    let hits = registry.query(query);
    for hit in hits.iter().take(15) {
        println!("{:<40} [{:?}, {}]", hit.item.title, hit.item.tier, hit.item.score);
    }
    eprintln!("\nscan en {scan_time:?}, {} résultats", hits.len());
    glib::ExitCode::SUCCESS
}
