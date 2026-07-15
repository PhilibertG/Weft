//! Provider fichiers, adossé à plocate.
//!
//! Choix délibéré : ne PAS écrire d'indexeur maison (un projet en soi) ni
//! se lier à Tracker/GNOME (l'OS final n'aura peut-être pas GNOME). plocate
//! est un outil système standard, requête en quelques millisecondes,
//! indexation gérée par le système (updatedb quotidien). S'il n'est pas
//! installé, le provider se tait — dégradation propre, jamais d'erreur.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::model::Icon;
use crate::provider::{Action, Provider, ResultItem, Tier};

/// En dessous de ce nombre de caractères, on ne cherche pas : trop de bruit
/// et l'utilisateur est probablement en train de taper un nom d'app.
const MIN_QUERY_LEN: usize = 3;
const MAX_FILES: usize = 6;

pub struct FilesProvider {
    /// Racine des résultats montrés (le home : les fichiers système dans
    /// /usr n'intéressent pas une recherche type Spotlight).
    home: Option<PathBuf>,
}

impl FilesProvider {
    pub fn new() -> Self {
        Self {
            home: std::env::var("HOME").map(PathBuf::from).ok(),
        }
    }
}

impl Default for FilesProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for FilesProvider {
    fn name(&self) -> &'static str {
        "files"
    }

    fn query(&mut self, query: &str) -> Vec<ResultItem> {
        let query = query.trim();
        if query.len() < MIN_QUERY_LEN {
            return Vec::new();
        }
        let Some(home) = &self.home else {
            return Vec::new();
        };

        // -i : insensible à la casse ; -l : plocate s'arrête tôt, la
        // sur-demande paie le filtrage (home, fichiers cachés) d'après.
        let Ok(output) = Command::new("plocate")
            .args(["-i", "-l", "40", "--", query])
            .output()
        else {
            return Vec::new(); // plocate absent : silence
        };
        if !output.status.success() {
            return Vec::new(); // base pas encore construite, etc.
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut items: Vec<ResultItem> = stdout
            .lines()
            .map(Path::new)
            .filter(|p| p.starts_with(home))
            .filter(|p| !hidden(p))
            .filter_map(|p| to_item(p, query))
            .collect();
        items.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.title.cmp(&b.title)));
        items.truncate(MAX_FILES);
        items
    }
}

/// Un composant caché n'importe où dans le chemin (~/.cache, ~/.config...).
fn hidden(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| s.starts_with('.') && s.len() > 1)
    })
}

fn to_item(path: &Path, query: &str) -> Option<ResultItem> {
    let name = path.file_name()?.to_string_lossy().into_owned();

    // plocate matche sur le chemin complet ; un match dans le nom du
    // fichier vaut plus qu'un match dans un répertoire parent, et un
    // chemin court vaut plus qu'un chemin profond.
    let in_name = name.to_lowercase().contains(&query.to_lowercase());
    let depth = path.components().count() as u32;
    let base: u32 = if in_name { 500 } else { 100 };
    let score = base.saturating_sub(depth.min(50));

    Some(ResultItem {
        id: format!("file:{}", path.display()),
        title: name,
        subtitle: Some(path.parent()?.display().to_string()),
        // Pas d'icône ici : l'UI la déduit du type de fichier (gio),
        // le cœur n'a pas à connaître les thèmes d'icônes.
        icon: path.is_dir().then(|| Icon::Named("folder".to_owned())),
        action: Action::OpenPath(path.to_path_buf()),
        uninstall: None,
        tier: Tier::Fallback,
        score,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_components_detected() {
        assert!(hidden(Path::new("/home/u/.cache/truc.txt")));
        assert!(!hidden(Path::new("/home/u/Documents/truc.txt")));
        // ".." ou "." purs ne comptent pas comme cachés.
        assert!(!hidden(Path::new("/home/u/Documents")));
    }

    #[test]
    fn filename_match_beats_path_match() {
        let by_name = to_item(Path::new("/home/u/Documents/rapport.pdf"), "rapport").unwrap();
        let by_path = to_item(Path::new("/home/u/rapport/notes.txt"), "rapport").unwrap();
        assert!(by_name.score > by_path.score);
        assert_eq!(by_name.tier, Tier::Fallback);
    }

    #[test]
    fn short_or_impossible_queries_are_silent() {
        let mut p = FilesProvider::new();
        assert!(p.query("ab").is_empty());
        assert!(p.query("").is_empty());
    }
}
