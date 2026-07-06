//! Scanner de la bibliothèque Steam.
//!
//! Steam décrit chaque jeu installé par un manifest texte
//! `steamapps/appmanifest_<appid>.acf` (format VDF, le "JSON de Valve").
//! Les bibliothèques secondaires (autres disques) sont listées dans
//! `steamapps/libraryfolders.vdf`. On lance toujours via le client Steam
//! (`steam://rungameid/<appid>`) : c'est lui qui gère Proton, donc les jeux
//! Windows marchent sans qu'on fasse quoi que ce soit de spécial.

use std::path::{Path, PathBuf};

use keyvalues_parser::{Value, Vdf};

use crate::model::{AppEntry, Icon, LaunchSpec, Source};
use crate::sources::AppSource;

/// Manifest dont StateFlags contient ce bit => jeu complètement installé.
const STATE_FULLY_INSTALLED: u64 = 4;

pub struct SteamScanner {
    /// Racine de l'installation Steam (contient steamapps/, appcache/...).
    root: Option<PathBuf>,
}

/// Installation Steam trouvée aux emplacements connus, s'il y en a une.
pub fn find_steam_root() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let home = Path::new(&home);
    let candidates = [
        home.join(".steam/steam"),
        home.join(".local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ];
    candidates.into_iter().find(|p| p.join("steamapps").is_dir())
}

/// Les répertoires steamapps/ de toutes les bibliothèques (watch inotify).
pub fn steamapps_dirs() -> Vec<PathBuf> {
    match find_steam_root() {
        Some(root) => library_dirs(&root)
            .into_iter()
            .map(|lib| lib.join("steamapps"))
            .collect(),
        None => Vec::new(),
    }
}

impl SteamScanner {
    /// Cherche une installation Steam aux emplacements connus.
    pub fn new() -> Self {
        Self { root: find_steam_root() }
    }

    /// Racine explicite (tests/fixtures).
    pub fn with_root(root: PathBuf) -> Self {
        Self { root: Some(root) }
    }
}

impl AppSource for SteamScanner {
    fn name(&self) -> &'static str {
        "steam"
    }

    fn scan(&self) -> Vec<AppEntry> {
        let Some(root) = &self.root else {
            return Vec::new();
        };

        let mut entries = Vec::new();
        for library in library_dirs(root) {
            let Ok(read) = std::fs::read_dir(library.join("steamapps")) else {
                continue;
            };
            for e in read.flatten() {
                let path = e.path();
                let is_manifest = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("appmanifest_") && n.ends_with(".acf"));
                if !is_manifest {
                    continue;
                }
                if let Some(entry) = parse_manifest(&path, root) {
                    entries.push(entry);
                }
            }
        }
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        entries
    }
}

/// Bibliothèques Steam : la racine elle-même + celles de libraryfolders.vdf.
fn library_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    let Ok(text) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) else {
        return dirs;
    };
    let Ok(vdf) = keyvalues_parser::parse(&text).map(Vdf::from) else {
        return dirs;
    };
    let Some(obj) = vdf.value.get_obj() else {
        return dirs;
    };
    // Structure : { "0" { "path" "/..." ... } "1" { ... } }
    for values in obj.values() {
        for v in values {
            if let Value::Obj(lib) = v {
                if let Some(path) = lib.get("path").and_then(|vs| vs.first()).and_then(Value::get_str)
                {
                    let p = PathBuf::from(path);
                    if !dirs.contains(&p) {
                        dirs.push(p);
                    }
                }
            }
        }
    }
    dirs
}

fn parse_manifest(path: &Path, steam_root: &Path) -> Option<AppEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let vdf = keyvalues_parser::parse(&text).map(Vdf::from).ok()?;
    let app = vdf.value.get_obj()?;

    let field = |key: &str| -> Option<&str> {
        app.get(key).and_then(|vs| vs.first()).and_then(Value::get_str)
    };

    let app_id: u32 = field("appid")?.parse().ok()?;
    let name = field("name")?.to_owned();
    let state: u64 = field("StateFlags").and_then(|s| s.parse().ok()).unwrap_or(0);

    if state & STATE_FULLY_INSTALLED == 0 {
        return None;
    }
    if is_tooling(&name) {
        return None;
    }

    Some(AppEntry {
        id: format!("steam:{app_id}"),
        name,
        description: None,
        icon: find_icon(steam_root, app_id),
        launch: LaunchSpec::SteamApp(app_id),
        source: Source::Steam,
        keywords: Vec::new(),
    })
}

/// Les manifests couvrent aussi l'outillage Steam (Proton, runtimes,
/// redistribuables) : ce ne sont pas des jeux, on les masque.
fn is_tooling(name: &str) -> bool {
    name.starts_with("Proton")
        || name.starts_with("Steam Linux Runtime")
        || name.starts_with("Steamworks Common")
}

/// Icône en cache local, best-effort (ancien et nouveau layout Steam).
/// Pas d'icône trouvée => l'UI affichera un fallback générique.
fn find_icon(steam_root: &Path, app_id: u32) -> Option<Icon> {
    let candidates = [
        steam_root.join(format!("appcache/librarycache/{app_id}_icon.jpg")),
        steam_root.join(format!("appcache/librarycache/{app_id}/icon.jpg")),
    ];
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .map(Icon::Path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooling_is_filtered() {
        assert!(is_tooling("Proton 9.0 (Beta)"));
        assert!(is_tooling("Steam Linux Runtime 3.0 (sniper)"));
        assert!(is_tooling("Steamworks Common Redistributables"));
        assert!(!is_tooling("Portal 2"));
    }
}
