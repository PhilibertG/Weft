//! CLI du moteur Windows (étape 2.1). Sert à valider le moteur avant
//! l'UX intégrée (2.2). Sous-commandes : runtime status|fetch, puis
//! install/list/remove/run aux incréments suivants.

use std::path::Path;
use std::process::ExitCode;

use weft_core::windows::prefix::AppStore;
use weft_core::windows::runtime::{Runtime, PINNED_PROTON, PINNED_UMU};
use weft_core::windows::{InstallOptions, WindowsEngine, WindowsRoot};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = WindowsRoot::open_default() else {
        eprintln!("weft-windows : HOME introuvable");
        return ExitCode::FAILURE;
    };

    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["runtime", "status"] => runtime_status(root),
        ["runtime", "fetch"] => runtime_fetch(root),
        ["list"] => list(root),
        ["remove", slug] => remove(root, slug),
        ["install", file] => install(root, file, None),
        ["install", file, "--gameid", gameid] => install(root, file, Some(gameid.to_owned())),
        ["run", slug] => run(root, slug),
        ["epic", "login"] => epic_login(),
        ["epic", "list"] => epic_list(),
        ["epic", "install", app_name] => epic_install(root, app_name),
        ["refresh-icons"] => {
            let done = WindowsEngine::new(root).refresh_icons();
            println!("Icônes extraites : {}", if done.is_empty() { "aucune".to_owned() } else { done.join(", ") });
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!(
                "usage: weft-windows <runtime status|runtime fetch|install <fichier> [--gameid <id>]|list|run <app>|remove <app>|refresh-icons|epic login|epic list|epic install <jeu>>"
            );
            ExitCode::FAILURE
        }
    }
}

fn epic_login() -> ExitCode {
    if !weft_core::windows::epic::available() {
        eprintln!("weft-windows : outil legendary manquant (pipx install legendary-gl)");
        return ExitCode::FAILURE;
    }
    // Flux interactif de legendary (navigateur + code à coller) : on lui
    // passe le terminal.
    match std::process::Command::new("legendary").arg("auth").status() {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

fn epic_list() -> ExitCode {
    match weft_core::windows::epic::library() {
        Ok(games) => {
            for g in &games {
                println!("{:<40} {}", g.app_name, g.title);
            }
            eprintln!("\n{} jeux — installer : weft-windows epic install <premier champ>", games.len());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("weft-windows : {e}");
            ExitCode::FAILURE
        }
    }
}

fn epic_install(root: WindowsRoot, app_name: &str) -> ExitCode {
    match WindowsEngine::new(root).install_epic(app_name, |msg| println!("{msg}")) {
        Ok(app) => {
            println!("OK : « {} » — visible dans le launcher.", app.manifest.name);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("weft-windows : {e}");
            ExitCode::FAILURE
        }
    }
}

fn install(root: WindowsRoot, file: &str, gameid: Option<String>) -> ExitCode {
    let opts = InstallOptions { gameid, ..Default::default() };
    match WindowsEngine::new(root).install(Path::new(file), opts, |msg| println!("{msg}")) {
        Ok(app) => {
            println!("OK : « {} » (slug {}) — visible dans le launcher.", app.manifest.name, app.slug);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("weft-windows : {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(root: WindowsRoot, slug: &str) -> ExitCode {
    match WindowsEngine::new(root).launch(slug) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("weft-windows : {e}");
            ExitCode::FAILURE
        }
    }
}

fn list(root: WindowsRoot) -> ExitCode {
    let apps = AppStore::new(root).list();
    if apps.is_empty() {
        println!("Aucune app Windows installée.");
        return ExitCode::SUCCESS;
    }
    for app in apps {
        println!(
            "{:<24} {:<40} [{}  gameid={}  {}]",
            app.slug,
            app.manifest.name,
            app.manifest.runtime.proton,
            app.manifest.gameid_or_default(),
            app.manifest.created,
        );
    }
    ExitCode::SUCCESS
}

fn remove(root: WindowsRoot, slug: &str) -> ExitCode {
    match AppStore::new(root).remove(slug) {
        Ok(()) => {
            println!("{slug} supprimée (préfixe compris).");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("weft-windows : {e}");
            ExitCode::FAILURE
        }
    }
}

fn runtime_status(root: WindowsRoot) -> ExitCode {
    let s = Runtime::new(root).status();
    let mark = |b: bool| if b { "✓" } else { "✗" };
    println!("umu {PINNED_UMU}          {}", mark(s.umu));
    println!("{PINNED_PROTON}   {}", mark(s.proton));
    println!("conteneur SLR       {}", mark(s.container));
    println!("python3             {}", mark(s.python));
    println!("prêt : {}", if s.ready() { "oui" } else { "non" });
    ExitCode::SUCCESS
}

fn runtime_fetch(root: WindowsRoot) -> ExitCode {
    let rt = Runtime::new(root);
    if let Err(e) = rt.fetch(|msg| println!("{msg}")) {
        eprintln!("weft-windows : {e}");
        return ExitCode::FAILURE;
    }
    if !rt.status().container {
        if let Err(e) = rt.fetch_container(|msg| println!("{msg}")) {
            eprintln!("weft-windows : {e}");
            return ExitCode::FAILURE;
        }
    }
    println!("Runtime prêt.");
    ExitCode::SUCCESS
}
