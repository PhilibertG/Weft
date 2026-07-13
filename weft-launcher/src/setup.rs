//! Assistant de premier lancement.
//!
//! Le paquet .deb pose les fichiers système ; ce que root n'a pas le droit
//! de toucher (service de la session utilisateur, raccourci clavier,
//! mimetypes par-utilisateur) est configuré ici, au premier lancement de
//! weft-launcher (ou à la demande via `--setup`). Tout est idempotent :
//! réexécuter ne casse ni ne duplique rien. Rien n'est imposé — chaque
//! étape est une action que l'utilisateur déclenche.

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc;

use adw::prelude::*;
use gtk::glib;

const SETUP_APP_ID: &str = "dev.weft.Setup";

/// Combinaison clavier proposée par défaut pour ouvrir le launcher.
const DEFAULT_SHORTCUT: &str = "<Super>Return";
const DEFAULT_SHORTCUT_LABEL: &str = "Super + Entrée";

const KEYBIND_ROOT: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings";

const WINDOWS_MIMES: &[&str] = &[
    "application/vnd.microsoft.portable-executable",
    "application/x-ms-dos-executable",
    "application/x-msdownload",
    "application/x-msi",
];

/// Fichier-marqueur : sa présence signifie « premier lancement déjà fait ».
pub fn marker_path() -> Option<PathBuf> {
    weft_core::config::config_path().map(|p| p.with_file_name(".setup-done"))
}

/// L'assistant doit-il s'ouvrir tout seul ? (premier lancement)
pub fn is_first_run() -> bool {
    marker_path().is_none_or(|p| !p.exists())
}

fn write_marker() {
    if let Some(p) = marker_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&p, "1\n");
    }
}

/// Lance l'assistant (fenêtre GTK) et rend le code de sortie.
pub fn run() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(SETUP_APP_ID)
        .build();
    app.connect_startup(|_| {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
        crate::load_css();
    });
    app.connect_activate(build_window);
    app.run_with_args::<String>(&[])
}

fn build_window(app: &adw::Application) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("weft-root");
    content.set_halign(gtk::Align::Center);
    content.set_valign(gtk::Align::Center);
    content.set_size_request(560, -1);

    let title = gtk::Label::new(Some("Bienvenue dans Weft"));
    title.add_css_class("weft-setup-title");
    title.set_halign(gtk::Align::Start);
    let subtitle = gtk::Label::new(Some(
        "Quelques réglages pour intégrer Weft à votre session. \
         Tout est facultatif et réversible.",
    ));
    subtitle.add_css_class("weft-desc");
    subtitle.set_halign(gtk::Align::Start);
    subtitle.set_wrap(true);
    subtitle.set_xalign(0.0);

    let header = gtk::Box::new(gtk::Orientation::Vertical, 4);
    header.set_margin_top(24);
    header.set_margin_start(24);
    header.set_margin_end(24);
    header.set_margin_bottom(8);
    header.append(&title);
    header.append(&subtitle);
    content.append(&header);

    let steps = gtk::Box::new(gtk::Orientation::Vertical, 6);
    steps.set_margin_start(16);
    steps.set_margin_end(16);
    steps.append(&service_step());
    steps.append(&shortcut_step());
    steps.append(&mime_step());
    steps.append(&apparmor_step());
    steps.append(&runtime_step());
    content.append(&steps);

    let done = gtk::Button::with_label("Terminer");
    done.add_css_class("weft-setup-primary");
    done.set_halign(gtk::Align::End);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    footer.set_margin_end(24);
    footer.set_margin_top(8);
    footer.set_margin_bottom(24);
    footer.set_hexpand(true);
    footer.append(&done);
    content.append(&footer);

    let backdrop = gtk::Box::new(gtk::Orientation::Vertical, 0);
    backdrop.append(&content);

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Weft — Premier lancement")
        .resizable(false)
        .child(&backdrop)
        .build();
    window.add_css_class("weft-window");

    done.connect_clicked(glib::clone!(
        #[weak] window,
        move |_| {
            write_marker();
            window.close();
        }
    ));

    window.present();
}

/// Une ligne d'étape : titre, description, état à droite, bouton d'action.
/// `check` renvoie Some(message) si l'étape est déjà satisfaite.
/// `action` est exécutée au clic et renvoie Ok(message) / Err(message).
struct Step {
    row: gtk::Box,
    status: gtk::Label,
    button: gtk::Button,
}

fn step_row(title: &str, desc: &str, action_label: &str) -> Step {
    let name = gtk::Label::new(Some(title));
    name.add_css_class("weft-name");
    name.set_halign(gtk::Align::Start);

    let description = gtk::Label::new(Some(desc));
    description.add_css_class("weft-desc");
    description.set_halign(gtk::Align::Start);
    description.set_wrap(true);
    description.set_xalign(0.0);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.append(&name);
    text.append(&description);

    let status = gtk::Label::new(None);
    status.add_css_class("weft-setup-status");
    status.set_valign(gtk::Align::Center);

    let button = gtk::Button::with_label(action_label);
    button.add_css_class("weft-setup-action");
    button.set_valign(gtk::Align::Center);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("weft-setup-step");
    row.append(&text);
    row.append(&status);
    row.append(&button);

    Step { row, status, button }
}

/// Marque une étape comme satisfaite (état vert, bouton masqué).
fn mark_done(status: &gtk::Label, button: &gtk::Button, msg: &str) {
    status.set_text(msg);
    status.remove_css_class("weft-status-todo");
    status.add_css_class("weft-status-done");
    button.set_visible(false);
}

fn mark_todo(status: &gtk::Label, msg: &str) {
    status.set_text(msg);
    status.remove_css_class("weft-status-done");
    status.add_css_class("weft-status-todo");
}

fn mark_error(status: &gtk::Label, msg: &str) {
    status.set_text(msg);
    status.remove_css_class("weft-status-done");
    status.add_css_class("weft-status-todo");
}

// --------------------------------------------------------------------- //
// Étape 1 : service utilisateur                                         //
// --------------------------------------------------------------------- //

fn service_step() -> gtk::Box {
    let step = step_row(
        "Démarrage automatique",
        "Lance Weft en arrière-plan à chaque connexion, pour une ouverture instantanée.",
        "Activer",
    );
    let (status, button) = (step.status.clone(), step.button.clone());

    if service_enabled() {
        mark_done(&status, &button, "✓ activé");
    } else {
        mark_todo(&status, "à activer");
    }

    button.connect_clicked(glib::clone!(
        #[weak] status, #[weak] button,
        move |_| match enable_service() {
            Ok(()) => mark_done(&status, &button, "✓ activé"),
            Err(e) => mark_error(&status, &e),
        }
    ));
    step.row
}

fn service_enabled() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-enabled", "weft-launcher"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "enabled")
}

fn enable_service() -> Result<(), String> {
    let out = Command::new("systemctl")
        .args(["--user", "enable", "--now", "weft-launcher"])
        .output()
        .map_err(|_| "systemctl introuvable".to_owned())?;
    if service_enabled() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .last()
            .unwrap_or("échec")
            .to_owned())
    }
}

// --------------------------------------------------------------------- //
// Étape 2 : raccourci clavier (via dconf, jamais d'écrasement)          //
// --------------------------------------------------------------------- //

fn shortcut_step() -> gtk::Box {
    let step = step_row(
        "Raccourci clavier",
        "Ouvre Weft d'un geste. Proposé : Super + Entrée.",
        "Installer",
    );
    let (status, button) = (step.status.clone(), step.button.clone());

    if !dconf_available() {
        button.set_sensitive(false);
        mark_todo(&status, "GNOME non détecté");
    } else if shortcut_installed() {
        mark_done(&status, &button, "✓ installé");
    } else {
        mark_todo(&status, DEFAULT_SHORTCUT_LABEL);
    }

    button.connect_clicked(glib::clone!(
        #[weak] status, #[weak] button,
        move |_| match install_shortcut() {
            Ok(()) => mark_done(&status, &button, "✓ installé"),
            Err(e) => mark_error(&status, &e),
        }
    ));
    step.row
}

fn dconf_available() -> bool {
    Command::new("dconf").arg("--version").output().is_ok()
}

/// Liste des chemins de raccourcis personnalisés déjà déclarés.
fn existing_keybindings() -> Vec<String> {
    let Ok(out) = Command::new("dconf").args(["read", KEYBIND_ROOT]).output() else {
        return Vec::new();
    };
    parse_gvariant_list(&String::from_utf8_lossy(&out.stdout))
}

/// Parse une liste GVariant de chemins : `['/a/', '/b/']` → ["/a/", "/b/"].
fn parse_gvariant_list(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.is_empty() || s == "@as []" || s == "[]" {
        return Vec::new();
    }
    s.trim_matches(['[', ']'])
        .split(',')
        .map(|p| p.trim().trim_matches('\'').to_owned())
        .filter(|p| !p.is_empty())
        .collect()
}

fn dconf_read(path: &str) -> String {
    Command::new("dconf")
        .args(["read", path])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_matches('\'').to_owned())
        .unwrap_or_default()
}

/// Un raccourci Weft est-il déjà installé ? (idempotence : on repère notre
/// commande, quel que soit l'index dconf)
fn shortcut_installed() -> bool {
    existing_keybindings()
        .iter()
        .any(|path| dconf_read(&format!("{path}command")).contains("weft-launcher"))
}

fn install_shortcut() -> Result<(), String> {
    let existing = existing_keybindings();

    // Déjà là : ne rien refaire.
    if existing
        .iter()
        .any(|p| dconf_read(&format!("{p}command")).contains("weft-launcher"))
    {
        return Ok(());
    }

    // Conflit : la combinaison est déjà prise par un AUTRE raccourci custom.
    if existing
        .iter()
        .any(|p| dconf_read(&format!("{p}binding")) == DEFAULT_SHORTCUT)
    {
        return Err(format!(
            "{DEFAULT_SHORTCUT_LABEL} déjà utilisé — à régler à la main dans Paramètres"
        ));
    }

    // Premier index libre, sans réutiliser un chemin déjà listé.
    let mut n = 0;
    let new_path = loop {
        let candidate = format!("{KEYBIND_ROOT}/custom{n}/");
        if !existing.contains(&candidate) {
            break candidate;
        }
        n += 1;
    };

    // APPEND (jamais de remplacement) puis renseigne la nouvelle entrée.
    let mut list = existing.clone();
    list.push(new_path.clone());
    let list_literal = format!(
        "[{}]",
        list.iter()
            .map(|p| format!("'{p}'"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    dconf_write(KEYBIND_ROOT, &list_literal)?;
    dconf_write(&format!("{new_path}name"), "'Weft'")?;
    dconf_write(&format!("{new_path}command"), "'/usr/bin/weft-launcher'")?;
    dconf_write(&format!("{new_path}binding"), &format!("'{DEFAULT_SHORTCUT}'"))?;
    Ok(())
}

fn dconf_write(path: &str, value: &str) -> Result<(), String> {
    let ok = Command::new("dconf")
        .args(["write", path, value])
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        Ok(())
    } else {
        Err("écriture dconf refusée".to_owned())
    }
}

// --------------------------------------------------------------------- //
// Étape 3 : handler .exe/.msi                                           //
// --------------------------------------------------------------------- //

fn mime_step() -> gtk::Box {
    let step = step_row(
        "Programmes Windows",
        "Ouvrir un .exe ou .msi lancera l'installation via Weft.",
        "Associer",
    );
    let (status, button) = (step.status.clone(), step.button.clone());

    if mime_associated() {
        mark_done(&status, &button, "✓ associé");
    } else {
        mark_todo(&status, "à associer");
    }

    button.connect_clicked(glib::clone!(
        #[weak] status, #[weak] button,
        move |_| match associate_mimes() {
            Ok(()) => mark_done(&status, &button, "✓ associé"),
            Err(e) => mark_error(&status, &e),
        }
    ));
    step.row
}

fn mime_associated() -> bool {
    Command::new("xdg-mime")
        .args(["query", "default", WINDOWS_MIMES[0]])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "weft-installer.desktop")
}

fn associate_mimes() -> Result<(), String> {
    let mut cmd = Command::new("xdg-mime");
    cmd.args(["default", "weft-installer.desktop"]);
    cmd.args(WINDOWS_MIMES);
    let ok = cmd.status().is_ok_and(|s| s.success());
    if ok && mime_associated() {
        Ok(())
    } else {
        Err("xdg-mime a échoué".to_owned())
    }
}

// --------------------------------------------------------------------- //
// Étape 4 : vérification AppArmor (informative)                         //
// --------------------------------------------------------------------- //

fn apparmor_step() -> gtk::Box {
    let step = step_row(
        "Isolation des jeux Windows",
        "Le profil AppArmor (posé par le paquet) autorise le conteneur d'exécution.",
        "Vérifier",
    );
    let (status, button) = (step.status.clone(), step.button.clone());

    if apparmor_profile_present() {
        mark_done(&status, &button, "✓ en place");
    } else {
        // Absence de fichier = AppArmor probablement désactivé sur la
        // machine : ce n'est pas une erreur bloquante.
        mark_todo(&status, "non requis ici");
        button.set_visible(false);
    }
    step.row
}

fn apparmor_profile_present() -> bool {
    std::path::Path::new("/etc/apparmor.d/weft-umu").is_file()
}

// --------------------------------------------------------------------- //
// Étape 5 : runtime Windows (téléchargement optionnel)                  //
// --------------------------------------------------------------------- //

fn runtime_step() -> gtk::Box {
    let step = step_row(
        "Environnement Windows",
        "Nécessaire pour les jeux et programmes Windows. ~1 Go, une seule fois.",
        "Télécharger",
    );
    let (status, button) = (step.status.clone(), step.button.clone());

    if runtime_ready() {
        mark_done(&status, &button, "✓ prêt");
    } else {
        mark_todo(&status, "plus tard, ou :");
    }

    button.connect_clicked(glib::clone!(
        #[weak] status, #[weak] button,
        move |_| {
            button.set_sensitive(false);
            mark_todo(&status, "téléchargement…");
            start_runtime_fetch(&status, &button);
        }
    ));
    step.row
}

fn runtime_ready() -> bool {
    weft_core::windows::WindowsRoot::open_default()
        .map(|root| weft_core::windows::runtime::Runtime::new(root).status().ready())
        .unwrap_or(false)
}

/// Télécharge le runtime dans un thread, l'UI reste vivante ; un canal
/// remonte l'avancement et le résultat au thread graphique.
fn start_runtime_fetch(status: &gtk::Label, button: &gtk::Button) {
    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    std::thread::spawn(move || {
        let Some(root) = weft_core::windows::WindowsRoot::open_default() else {
            let _ = tx.send(Err("dossier utilisateur introuvable".into()));
            return;
        };
        let rt = weft_core::windows::runtime::Runtime::new(root);
        let tx2 = tx.clone();
        let result = rt
            .fetch(|m| {
                let _ = tx2.send(Ok(format!("… {m}")));
            })
            .and_then(|_| rt.fetch_container(|m| {
                let _ = tx2.send(Ok(format!("… {m}")));
            }));
        let _ = tx.send(match result {
            Ok(()) => Ok("done".into()),
            Err(e) => Err(e.to_string()),
        });
    });

    let status = status.clone();
    let button = button.clone();
    let rx = Rc::new(RefCell::new(rx));
    glib::timeout_add_local(std::time::Duration::from_millis(120), move || {
        let mut finished = false;
        while let Ok(msg) = rx.borrow().try_recv() {
            match msg {
                Ok(m) if m == "done" => {
                    mark_done(&status, &button, "✓ prêt");
                    finished = true;
                }
                Ok(m) => {
                    // Message d'avancement : tronqué pour rester lisible.
                    let short: String = m.chars().take(28).collect();
                    status.set_text(&short);
                }
                Err(e) => {
                    button.set_sensitive(true);
                    mark_error(&status, &format!("échec : {e}"));
                    finished = true;
                }
            }
        }
        if finished {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}
