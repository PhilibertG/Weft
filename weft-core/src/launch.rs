//! Lancement des apps, détaché du launcher.
//!
//! Le process lancé est mis dans son propre groupe de processus et ses
//! flux sont coupés : si le launcher se ferme (ou crashe), l'app survit.

use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::model::LaunchSpec;

pub fn launch_spec(spec: &LaunchSpec) -> io::Result<()> {
    match spec {
        LaunchSpec::Exec(argv) => spawn_detached(argv),
        // App Windows Weft : le moteur reconstruit l'environnement
        // (préfixe isolé, Proton épinglé, gameid) et logue dans l'app.
        LaunchSpec::WindowsApp(slug) => {
            let root = crate::windows::WindowsRoot::open_default()
                .ok_or_else(|| io::Error::other("HOME introuvable"))?;
            crate::windows::WindowsEngine::new(root).launch(slug)
        }
        // Le client Steam gère tout (Proton compris). S'il ne tourne pas,
        // la commande le démarre puis lance le jeu. Ordre : client natif,
        // client Flatpak (pas de binaire `steam` sur le PATH dans ce cas),
        // et xdg-open en dernier recours (handler steam:// du bureau).
        LaunchSpec::SteamApp(app_id) => {
            let url = format!("steam://rungameid/{app_id}");
            if spawn_detached(&["steam".to_owned(), url.clone()]).is_ok() {
                return Ok(());
            }
            if flatpak_steam_installed() {
                spawn_detached(&[
                    "flatpak".to_owned(),
                    "run".to_owned(),
                    "com.valvesoftware.Steam".to_owned(),
                    url,
                ])
            } else {
                spawn_detached(&["xdg-open".to_owned(), url])
            }
        }
    }
}

/// Ouvre un fichier/dossier avec l'application par défaut.
pub fn open_path(path: &Path) -> io::Result<()> {
    spawn_detached(&["xdg-open".to_owned(), path.display().to_string()])
}

fn flatpak_steam_installed() -> bool {
    std::env::var("HOME").is_ok_and(|h| {
        Path::new(&h)
            .join(".var/app/com.valvesoftware.Steam")
            .is_dir()
    })
}

fn spawn_detached<S: AsRef<str>>(argv: &[S]) -> io::Result<()> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "argv vide"))?;
    Command::new(program.as_ref())
        .args(args.iter().map(|a| a.as_ref()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(())
}
