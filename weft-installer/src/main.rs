//! UI d'installation d'un programme Windows (étape 2.2).
//!
//! `weft-installer <fichier.exe|.msi>` : fenêtre minimale qui suit la
//! progression, l'assistant Windows du programme s'affiche au milieu,
//! et à la fin l'app est dans le launcher. Aucun jargon : l'utilisateur
//! ne voit ni Wine, ni Proton, ni préfixe. Échec => message honnête,
//! jamais de stacktrace.
//!
//! Si le support Windows n'a jamais été initialisé (première fois), le
//! téléchargement (~1 Go) est proposé et suivi dans la même fenêtre —
//! c'est l'UX branchée sur RuntimeStatus/fetch() du moteur.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

use weft_core::windows::{WindowsEngine, WindowsRoot};

const APP_ID: &str = "dev.weft.Installer";

enum Event {
    Progress(String),
    Done(Result<String, String>),
}

fn main() -> glib::ExitCode {
    let Some(file) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: weft-installer <fichier.exe|.msi>");
        return glib::ExitCode::FAILURE;
    };

    let app = adw::Application::builder()
        .application_id(APP_ID)
        // Plusieurs installations en parallèle = plusieurs fenêtres.
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| build_ui(app, file.clone()));
    app.run_with_args::<String>(&[])
}

fn build_ui(app: &adw::Application, file: PathBuf) {
    let file_label = file
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();

    let title = gtk::Label::builder()
        .label(format!("Installation de {file_label}"))
        .build();
    title.add_css_class("title-2");

    let status = gtk::Label::builder()
        .label("Préparation…")
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    status.add_css_class("dim-label");

    let spinner = gtk::Spinner::builder().spinning(true).build();
    spinner.set_size_request(32, 32);

    let close = gtk::Button::with_label("Fermer");
    close.set_sensitive(false);
    close.set_halign(gtk::Align::Center);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_margin_top(28);
    content.set_margin_bottom(28);
    content.set_margin_start(36);
    content.set_margin_end(36);
    content.append(&title);
    content.append(&spinner);
    content.append(&status);
    content.append(&close);

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Weft")
        .resizable(false)
        .default_width(420)
        .child(&content)
        .build();

    close.connect_clicked(glib::clone!(
        #[weak] window,
        move |_| window.close()
    ));

    // Moteur dans un thread : la fenêtre reste vivante pendant que
    // l'installeur Windows tourne.
    let (tx, rx) = mpsc::channel::<Event>();
    std::thread::spawn(move || run_install(file, tx));

    glib::timeout_add_local(
        Duration::from_millis(80),
        glib::clone!(
            #[weak] status, #[weak] spinner, #[weak] close, #[weak] title,
            #[upgrade_or] glib::ControlFlow::Break,
            move || {
                while let Ok(event) = rx.try_recv() {
                    match event {
                        Event::Progress(msg) => status.set_label(&msg),
                        Event::Done(Ok(name)) => {
                            spinner.set_spinning(false);
                            spinner.set_visible(false);
                            title.set_label(&format!("« {name} » est installé"));
                            status.set_label(
                                "Retrouvez-le dans votre launcher (Super+Entrée).",
                            );
                            close.set_sensitive(true);
                            return glib::ControlFlow::Break;
                        }
                        Event::Done(Err(msg)) => {
                            spinner.set_spinning(false);
                            spinner.set_visible(false);
                            title.set_label("Installation impossible");
                            status.set_label(&msg);
                            close.set_sensitive(true);
                            return glib::ControlFlow::Break;
                        }
                    }
                }
                glib::ControlFlow::Continue
            }
        ),
    );

    window.present();
}

fn run_install(file: PathBuf, tx: mpsc::Sender<Event>) {
    let send = |e: Event| {
        let _ = tx.send(e);
    };

    let Some(root) = WindowsRoot::open_default() else {
        send(Event::Done(Err("dossier utilisateur introuvable".into())));
        return;
    };
    let engine = WindowsEngine::new(root);

    // Premier usage : préparer le support Windows dans la même fenêtre.
    if !engine.runtime().status().ready() {
        send(Event::Progress(
            "Première utilisation : préparation du support Windows (~1 Go, une seule fois)…"
                .into(),
        ));
        let fetch = engine
            .runtime()
            .fetch(|m| send(Event::Progress(m.to_owned())))
            .and_then(|_| {
                engine
                    .runtime()
                    .fetch_container(|m| send(Event::Progress(m.to_owned())))
            });
        if let Err(e) = fetch {
            send(Event::Done(Err(format!(
                "le support Windows n'a pas pu être préparé : {e}"
            ))));
            return;
        }
    }

    let result = engine
        .install(&file, Default::default(), |m| send(Event::Progress(m.to_owned())))
        .map(|app| app.manifest.name)
        .map_err(|e| e.to_string());
    send(Event::Done(result));
}
