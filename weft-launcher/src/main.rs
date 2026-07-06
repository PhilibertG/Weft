//! UI overlay du launcher, style Spotlight.
//!
//! Fenêtre sans décorations, barre de recherche + résultats. GApplication
//! garantit l'instance unique : relancer le binaire (le raccourci clavier
//! GNOME fait exactement ça) réveille l'instance résidente via D-Bus, la
//! fenêtre réapparaît instantanément. Échap la cache sans tuer le process.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use gtk::pango;

use weft_core::model::{AppEntry, Icon};
use weft_core::search::Searcher;
use weft_core::Index;

const APP_ID: &str = "dev.weft.Launcher";
const MAX_RESULTS: usize = 8;

struct State {
    entries: Vec<AppEntry>,
    searcher: Searcher,
    /// Indices (dans `entries`) actuellement affichés, ordre = ListBox.
    hits: Vec<usize>,
}

fn main() -> glib::ExitCode {
    // Mode debug CLI conservé : `weft-launcher --list [requête]`.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--list") {
        return debug_list(args.get(2).map(String::as_str).unwrap_or(""));
    }

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| load_css());
    app.connect_activate(activate);
    // GApplication consommerait argv ; on ne lui passe rien.
    app.run_with_args::<String>(&[])
}

fn activate(app: &adw::Application) {
    // Deuxième invocation : la fenêtre existe déjà, on la remontre avec un
    // index rafraîchi (des apps ont pu être (dés)installées entre-temps).
    if let Some(window) = app.active_window() {
        refresh_and_present(&window);
        return;
    }
    build_ui(app);
}

fn build_ui(app: &adw::Application) {
    let state = Rc::new(RefCell::new(State {
        entries: Vec::new(),
        searcher: Searcher::new(),
        hits: Vec::new(),
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
    root.append(&entry);
    root.append(&scroller);

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Weft")
        .default_width(620)
        .default_height(440)
        .resizable(false)
        .decorated(false)
        .child(&root)
        .build();
    window.add_css_class("weft-window");
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
    refresh_and_present(window.upcast_ref());
}

/// Re-scanne le système, vide la recherche, montre la fenêtre.
fn refresh_and_present(window: &gtk::Window) {
    let (state, entry, list) = ui_parts(window);
    state.borrow_mut().entries = Index::build().into_entries();
    entry.set_text("");
    refresh_list(&state, &list, "");
    window.present();
    entry.grab_focus();
}

fn refresh_list(state: &Rc<RefCell<State>>, list: &gtk::ListBox, query: &str) {
    let mut s = state.borrow_mut();
    let State { entries, searcher, hits } = &mut *s;
    *hits = searcher.search(entries, query);
    // Requête vide : liste complète scrollable. Sinon, top résultats.
    if !query.is_empty() {
        hits.truncate(MAX_RESULTS);
    }

    list.remove_all();
    for &i in hits.iter() {
        list.append(&make_row(&entries[i]));
    }
    drop(s);
    // Sélection par défaut : premier résultat, prêt pour Entrée.
    list.select_row(list.row_at_index(0).as_ref());
}

fn make_row(app: &AppEntry) -> gtk::ListBoxRow {
    let icon = match &app.icon {
        Some(Icon::Named(name)) => gtk::Image::from_icon_name(name),
        Some(Icon::Path(path)) => gtk::Image::from_file(path),
        None => gtk::Image::from_icon_name("application-x-executable-symbolic"),
    };
    icon.set_pixel_size(32);

    let name = gtk::Label::builder()
        .label(&app.name)
        .halign(gtk::Align::Start)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    name.add_css_class("weft-name");

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    text.append(&name);
    if let Some(desc) = &app.description {
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

fn launch_row(state: &Rc<RefCell<State>>, row_index: i32, window: &gtk::ApplicationWindow) {
    let s = state.borrow();
    let Some(&entry_idx) = s.hits.get(row_index as usize) else { return };
    let app = &s.entries[entry_idx];
    match weft_core::launch::launch(app) {
        Ok(()) => {
            drop(s);
            window.close(); // hide_on_close => juste caché
        }
        Err(e) => eprintln!("weft: échec du lancement de « {} » : {e}", app.name),
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
    provider.load_from_string(
        "
        window.weft-window { background: transparent; }
        .weft-root {
            background-color: @window_bg_color;
            border-radius: 14px;
            border: 1px solid alpha(@borders, 0.6);
        }
        .weft-entry {
            margin: 12px;
            min-height: 44px;
            font-size: 17px;
        }
        .weft-list, .weft-list row { background: transparent; }
        .weft-row { padding: 8px 14px; border-radius: 10px; margin: 0 8px; }
        .weft-name { font-size: 15px; }
        .weft-desc { font-size: 12px; }
        ",
    );
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
    let index = Index::build();
    let scan_time = start.elapsed();
    let mut searcher = Searcher::new();
    let hits = searcher.search(index.entries(), query);
    for &i in hits.iter().take(15) {
        let e = &index.entries()[i];
        println!("{:<40} [{:?}]", e.name, e.source);
    }
    eprintln!("\n{} apps en {scan_time:?}, {} résultats", index.len(), hits.len());
    glib::ExitCode::SUCCESS
}
