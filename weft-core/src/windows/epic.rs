//! Intégration Epic Games Store via legendary (le CLI libre qui remplace
//! le launcher Epic — c'est aussi ce que Heroic utilise).
//!
//! Rôles : legendary gère le compte, le téléchargement et l'injection des
//! arguments d'authentification au lancement ; Weft garde tout le reste
//! (préfixe isolé, runtime épinglé, manifest, launcher). legendary est un
//! outil externe présence-testée, comme plocate : absent => fonctionnalité
//! muette, jamais d'erreur ailleurs.

use std::io;
use std::process::Command;

/// legendary est-il utilisable sur cette machine ?
pub fn available() -> bool {
    Command::new("legendary")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Un compte Epic est-il connecté ?
pub fn logged_in() -> bool {
    let Ok(out) = Command::new("legendary").args(["status", "--json"]).output() else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    serde_json::from_slice::<serde_json::Value>(&out.stdout)
        .ok()
        .and_then(|v| v.get("account").cloned())
        .is_some_and(|a| a.as_str().is_some_and(|s| s != "<not logged in>"))
}

#[derive(Debug, Clone)]
pub struct EpicGame {
    /// Identifiant interne Epic (app_name legendary) — le futur store_id.
    pub app_name: String,
    pub title: String,
}

/// La bibliothèque du compte (jeux possédés, installés ou non).
pub fn library() -> io::Result<Vec<EpicGame>> {
    let out = Command::new("legendary")
        .args(["list", "--json"])
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(
            "impossible de lire la bibliothèque Epic (compte connecté ?)",
        ));
    }
    let games: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout)
        .map_err(|e| io::Error::other(format!("réponse legendary illisible : {e}")))?;
    let mut list: Vec<EpicGame> = games
        .iter()
        .filter_map(|g| {
            Some(EpicGame {
                app_name: g.get("app_name")?.as_str()?.to_owned(),
                title: g.get("app_title")?.as_str()?.to_owned(),
            })
        })
        .collect();
    list.sort_by(|a, b| a.title.cmp(&b.title));
    Ok(list)
}

/// Pour un jeu installé : (répertoire d'installation absolu, exécutable
/// principal relatif à ce répertoire) — tels que déclarés par Epic.
pub fn installed_info(app_name: &str) -> Option<(std::path::PathBuf, String)> {
    let out = Command::new("legendary")
        .args(["list-installed", "--json"])
        .output()
        .ok()?;
    let games: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).ok()?;
    games.iter().find_map(|g| {
        if g.get("app_name")?.as_str()? != app_name {
            return None;
        }
        let path = g.get("install_path")?.as_str()?;
        let exe = g.get("executable")?.as_str()?;
        Some((
            std::path::PathBuf::from(path),
            exe.trim_start_matches(['/', '\\']).replace('\\', "/"),
        ))
    })
}

/// Cherche l'id protonfixes d'un jeu dans la base umu (best-effort :
/// hors-ligne ou jeu inconnu => None, on lancera en umu-default).
pub fn umu_id(app_name: &str, store: &str) -> Option<String> {
    let url = format!(
        "https://umu.openwinecomponents.org/umu_api.php?codename={app_name}&store={store}"
    );
    let out = Command::new("curl")
        .args(["-s", "--max-time", "10", &url])
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    // L'API répond une liste d'entrées { umu_id, ... }.
    v.as_array()?
        .first()?
        .get("umu_id")?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pas de compte Epic sur la machine de CI/test : on ne teste ici que
    // la dégradation propre. Le chemin nominal est validé en réel.
    #[test]
    fn degrades_quietly_without_legendary_or_account() {
        // Quoi qu'il y ait sur la machine, aucun panic possible.
        let _ = available();
        let _ = logged_in();
    }
}
