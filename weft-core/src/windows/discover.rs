//! Découverte de ce qu'un installeur a posé dans le préfixe : les .lnk du
//! menu Démarrer d'abord (ce que l'installeur considère comme SES apps),
//! heuristique "plus gros exe de Program Files" en secours.

use std::path::{Path, PathBuf};

/// Candidat au titre d'exe principal de l'app.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredExe {
    /// Nom lisible (celui du raccourci si .lnk, du fichier sinon).
    pub name: String,
    /// Chemin RELATIF au préfixe (stocké tel quel dans le manifest).
    pub exe: String,
}

/// Emplacements des menus Démarrer dans un préfixe Wine/Proton.
const START_MENU_DIRS: &[&str] = &[
    "drive_c/ProgramData/Microsoft/Windows/Start Menu",
    "drive_c/users/steamuser/AppData/Roaming/Microsoft/Windows/Start Menu",
    "drive_c/users/Public/Start Menu",
];

/// Ce qui n'est jamais l'app elle-même.
const NOISE: &[&str] = &["uninstall", "désinstall", "readme", "license", "website", "help"];

pub fn discover(prefix: &Path) -> Vec<DiscoveredExe> {
    let mut found = discover_from_lnk(prefix);
    if found.is_empty() {
        found = discover_from_program_files(prefix);
    }
    found
}

/// Les .lnk du menu Démarrer pointant vers des .exe du préfixe.
fn discover_from_lnk(prefix: &Path) -> Vec<DiscoveredExe> {
    let mut out = Vec::new();
    for dir in START_MENU_DIRS {
        for lnk_path in walk_ext(&prefix.join(dir), "lnk") {
            let Some(target) = lnk_target(&lnk_path) else { continue };
            let Some(rel) = windows_path_to_prefix_relative(&target) else { continue };
            if !rel.to_lowercase().ends_with(".exe") {
                continue;
            }
            let name = lnk_path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if is_noise(&name) || !prefix.join(&rel).is_file() {
                continue;
            }
            out.push(DiscoveredExe { name, exe: rel });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.exe == b.exe);
    out
}

/// Secours : le plus gros .exe sous Program Files (les installeurs sans
/// raccourci, ou les archives portables décompressées).
fn discover_from_program_files(prefix: &Path) -> Vec<DiscoveredExe> {
    let mut best: Option<(u64, DiscoveredExe)> = None;
    for pf in ["drive_c/Program Files", "drive_c/Program Files (x86)", "drive_c/weft-portable"] {
        for exe in walk_ext(&prefix.join(pf), "exe") {
            let name = exe
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if is_noise(&name) {
                continue;
            }
            let size = exe.metadata().map(|m| m.len()).unwrap_or(0);
            let rel = exe
                .strip_prefix(prefix)
                .unwrap_or(&exe)
                .to_string_lossy()
                .into_owned();
            if best.as_ref().is_none_or(|(s, _)| size > *s) {
                best = Some((size, DiscoveredExe { name, exe: rel }));
            }
        }
    }
    best.map(|(_, d)| vec![d]).unwrap_or_default()
}

/// `C:\Program Files\App\app.exe` → `drive_c/Program Files/App/app.exe`.
fn windows_path_to_prefix_relative(target: &str) -> Option<String> {
    let lower = target.to_lowercase();
    if !lower.starts_with("c:\\") {
        return None; // autre lecteur : hors préfixe, on ignore
    }
    Some(format!("drive_c/{}", target[3..].replace('\\', "/")))
}

fn lnk_target(path: &Path) -> Option<String> {
    let link = lnk::ShellLink::open(path, encoding_rs::WINDOWS_1252).ok()?;
    link.link_target()
}

fn is_noise(name: &str) -> bool {
    let lower = name.to_lowercase();
    NOISE.iter().any(|n| lower.contains(n))
}

fn walk_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else { continue };
        for e in read.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_prefix(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("weft-disc-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn touch(prefix: &Path, rel: &str, size: usize) {
        let p = prefix.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![0u8; size]).unwrap();
    }

    #[test]
    fn windows_paths_map_into_prefix() {
        assert_eq!(
            windows_path_to_prefix_relative(r"C:\Program Files\WinMerge\WinMergeU.exe").unwrap(),
            "drive_c/Program Files/WinMerge/WinMergeU.exe"
        );
        assert!(windows_path_to_prefix_relative(r"D:\jeu\jeu.exe").is_none());
    }

    #[test]
    fn fallback_picks_biggest_exe_and_skips_noise() {
        let prefix = temp_prefix("fallback");
        touch(&prefix, "drive_c/Program Files/App/app.exe", 5000);
        touch(&prefix, "drive_c/Program Files/App/petit-outil.exe", 100);
        touch(&prefix, "drive_c/Program Files/App/uninstall.exe", 90000);

        let found = discover(&prefix);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].exe, "drive_c/Program Files/App/app.exe");

        let _ = std::fs::remove_dir_all(&prefix);
    }

    #[test]
    fn empty_prefix_discovers_nothing() {
        let prefix = temp_prefix("empty");
        assert!(discover(&prefix).is_empty());
        let _ = std::fs::remove_dir_all(&prefix);
    }
}
