//! Intégration GOG via lgogdownloader (dépôts Ubuntu).
//!
//! Les jeux GOG sont DRM-free, distribués en installeurs offline Inno
//! Setup : lgogdownloader télécharge l'installeur officiel, et le moteur
//! 2.2 existant l'installe en silencieux dans son préfixe isolé. Aucun
//! client GOG. Outil présence-testé : absent => fonctionnalité muette.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn available() -> bool {
    Command::new("lgogdownloader")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Un compte GOG est-il connecté ? (lgogdownloader garde ses jetons dans
/// ~/.config/lgogdownloader/)
pub fn logged_in() -> bool {
    let Ok(home) = std::env::var("HOME") else { return false };
    let cfg = Path::new(&home).join(".config/lgogdownloader");
    cfg.join("galaxy_tokens.json").is_file() || cfg.join("cookies.txt").is_file()
}

#[derive(Debug, Clone)]
pub struct GogGame {
    /// Slug GOG ("betrayer", "stardew_valley") — le futur store_id.
    pub gamename: String,
    pub title: String,
    /// Product id numérique (clé de la base protonfixes pour STORE=gog).
    pub product_id: String,
}

/// La bibliothèque du compte.
pub fn library() -> io::Result<Vec<GogGame>> {
    let out = Command::new("lgogdownloader")
        .args(["--list", "json"])
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(
            "impossible de lire la bibliothèque GOG (compte connecté ?)",
        ));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| io::Error::other(format!("réponse lgogdownloader illisible : {e}")))?;
    let games = v
        .get("games")
        .and_then(|g| g.as_array())
        .cloned()
        .or_else(|| v.as_array().cloned())
        .unwrap_or_default();
    let mut list: Vec<GogGame> = games
        .iter()
        .filter_map(|g| {
            Some(GogGame {
                gamename: g.get("gamename")?.as_str()?.to_owned(),
                title: g
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or(g.get("gamename")?.as_str()?)
                    .to_owned(),
                product_id: match g.get("product_id").or_else(|| g.get("id")) {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(serde_json::Value::Number(n)) => n.to_string(),
                    _ => String::new(),
                },
            })
        })
        .collect();
    list.sort_by(|a, b| a.title.cmp(&b.title));
    Ok(list)
}

/// Télécharge l'installeur offline Windows d'un jeu dans `dest` et
/// retourne le chemin du setup_*.exe principal (ses éventuels .bin
/// restent à côté, l'installeur les trouve tout seul).
pub fn download_installer(gamename: &str, dest: &Path, log: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dest)?;
    let log_file = std::fs::File::create(log)?;
    let status = Command::new("lgogdownloader")
        .args([
            "--download",
            "--game",
            &format!("^{}$", regex_escape(gamename)),
            "--platform",
            "w",
            "--include",
            "basegame_installers",
            "--directory",
        ])
        .arg(dest)
        .stdin(Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file)
        .status()?;
    if !status.success() {
        return Err(io::Error::other("le téléchargement GOG a échoué (connexion ?)"));
    }

    // Les fichiers arrivent sous dest/<gamename>/ : setup_*.exe + .bin.
    find_setup_exe(&dest.join(gamename))
        .or_else(|| find_setup_exe(dest))
        .ok_or_else(|| io::Error::other("installeur téléchargé mais introuvable"))
}

fn find_setup_exe(dir: &Path) -> Option<PathBuf> {
    let mut exes: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe"))
        })
        .collect();
    exes.sort();
    exes.into_iter().next()
}

/// Échappe un slug pour l'utiliser dans le filtre regex de lgogdownloader.
fn regex_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                vec![c]
            } else {
                vec!['\\', c]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degrades_quietly_without_tool_or_account() {
        let _ = available();
        let _ = logged_in();
    }

    #[test]
    fn setup_exe_is_found_next_to_bin_parts() {
        let dir = std::env::temp_dir().join(format!("weft-gog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("setup_betrayer_1.9.exe"), b"MZ").unwrap();
        std::fs::write(dir.join("setup_betrayer_1.9-1.bin"), b"data").unwrap();

        assert_eq!(
            find_setup_exe(&dir).unwrap().file_name().unwrap(),
            "setup_betrayer_1.9.exe"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn regex_escape_keeps_slugs_and_escapes_rest() {
        assert_eq!(regex_escape("stardew_valley"), "stardew_valley");
        assert_eq!(regex_escape("a.b+c"), "a\\.b\\+c");
    }
}
