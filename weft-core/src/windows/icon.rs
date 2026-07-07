//! Extraction de l'icône d'un exécutable Windows (ressources PE) vers un
//! PNG, via icoutils (wrestool + icotool). Best-effort intégral : outil
//! absent, exe sans icône, ressource corrompue => pas d'icône, jamais
//! d'erreur bloquante. L'UI a son fallback.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Extrait la meilleure icône de `exe` vers `dest_png`.
/// `true` si un PNG a été écrit.
pub fn extract_icon(exe: &Path, dest_png: &Path) -> bool {
    let Some(work) = tmp_dir(dest_png) else { return false };
    let ok = try_extract(exe, dest_png, &work);
    let _ = std::fs::remove_dir_all(&work);
    ok
}

fn try_extract(exe: &Path, dest_png: &Path, work: &Path) -> bool {
    // 1. Sortir les groupes d'icônes (.ico) des ressources PE.
    let out = Command::new("wrestool")
        .arg("-x")
        .arg("--type=14") // RT_GROUP_ICON
        .arg(format!("--output={}", work.display()))
        .arg(exe)
        .output();
    if !matches!(out, Ok(o) if o.status.success()) {
        return false; // outil absent ou exe sans ressources
    }
    let Some(ico) = biggest_file(work, "ico") else { return false };

    // 2. Éclater le .ico en PNGs (une image par taille).
    let pngs = work.join("pngs");
    if std::fs::create_dir_all(&pngs).is_err() {
        return false;
    }
    let out = Command::new("icotool")
        .arg("-x")
        .arg("-o")
        .arg(&pngs)
        .arg(&ico)
        .output();
    if !matches!(out, Ok(o) if o.status.success()) {
        return false;
    }

    // 3. Garder la plus grande image.
    let Some(best) = biggest_file(&pngs, "png") else { return false };
    std::fs::copy(&best, dest_png).is_ok()
}

/// Le plus gros fichier `.ext` du répertoire — pour un .ico ou un PNG,
/// « plus gros » approxime bien « plus haute résolution ».
fn biggest_file(dir: &Path, ext: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)))
        .max_by_key(|p| p.metadata().map(|m| m.len()).unwrap_or(0))
}

fn tmp_dir(dest: &Path) -> Option<PathBuf> {
    let dir = dest.parent()?.join(".icon-extract");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tool_or_bad_exe_degrades_to_false() {
        let dir = std::env::temp_dir().join(format!("weft-icon-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake_exe = dir.join("fake.exe");
        std::fs::write(&fake_exe, b"MZ pas un vrai PE").unwrap();

        // Quel que soit l'état d'icoutils sur la machine : pas de panique,
        // pas de PNG.
        let dest = dir.join("icon.png");
        assert!(!extract_icon(&fake_exe, &dest));
        assert!(!dest.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
