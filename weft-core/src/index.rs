//! Agrégation des sources en un index unique et dédupliqué.

use std::collections::HashSet;

use crate::model::{AppEntry, Source};
use crate::sources::AppSource;

pub struct Index {
    entries: Vec<AppEntry>,
}

impl Index {
    /// Construit l'index avec les sources par défaut du système réel.
    pub fn build() -> Self {
        use crate::sources::{
            desktop::DesktopScanner, steam::SteamScanner, windows::WindowsAppsScanner,
        };
        let desktop = DesktopScanner::new();
        let steam = SteamScanner::new();
        let mut sources: Vec<&dyn AppSource> = vec![&desktop, &steam];
        let windows = WindowsAppsScanner::new();
        if let Some(w) = &windows {
            sources.push(w);
        }
        Self::from_sources(&sources)
    }

    /// Construit l'index à partir de sources arbitraires (tests).
    ///
    /// Déduplication par id : un jeu Steam vu à la fois par son manifest et
    /// par un raccourci .desktop (`steam://rungameid/<appid>`) porte le même
    /// id `steam:<appid>`, la première occurrence gagne.
    pub fn from_sources(sources: &[&dyn AppSource]) -> Self {
        let mut seen: HashSet<String> = HashSet::new();
        let mut entries = Vec::new();
        for source in sources {
            for entry in source.scan() {
                if seen.insert(entry.id.clone()) {
                    entries.push(entry);
                }
            }
        }
        Self {
            entries: dedup_native_flatpak(entries),
        }
    }

    pub fn entries(&self) -> &[AppEntry] {
        &self.entries
    }

    pub fn into_entries(self) -> Vec<AppEntry> {
        self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Même app en .deb ET en Flatpak => une seule entrée, priorité au natif
/// (démarrage plus rapide, mieux intégré ; l'utilisateur qui préfère le
/// Flatpak désinstalle le .deb). Rapprochement par nom normalisé — les ids
/// diffèrent toujours (`firefox.desktop` vs `org.mozilla.firefox.desktop`).
fn dedup_native_flatpak(entries: Vec<AppEntry>) -> Vec<AppEntry> {
    let native_names: HashSet<String> = entries
        .iter()
        .filter(|e| e.source == Source::Native)
        .map(|e| normalize_name(&e.name))
        .collect();
    entries
        .into_iter()
        .filter(|e| e.source != Source::Flatpak || !native_names.contains(&normalize_name(&e.name)))
        .collect()
}

/// "Notepad++" et "notepad++", "GIMP " et "GIMP" doivent se rapprocher.
fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
