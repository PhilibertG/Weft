//! Lancement des apps, détaché du launcher.
//!
//! Le process lancé est mis dans son propre groupe de processus et ses
//! flux sont coupés : si le launcher se ferme (ou crashe), l'app survit.

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::model::{AppEntry, LaunchSpec};

pub fn launch(entry: &AppEntry) -> io::Result<()> {
    match &entry.launch {
        LaunchSpec::Exec(argv) => spawn_detached(argv),
        // Le client Steam gère tout (Proton compris). S'il ne tourne pas,
        // la commande `steam` le démarre puis lance le jeu.
        LaunchSpec::SteamApp(app_id) => {
            let url = format!("steam://rungameid/{app_id}");
            spawn_detached(&["steam".to_owned(), url.clone()])
                .or_else(|_| spawn_detached(&["xdg-open".to_owned(), url]))
        }
    }
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
