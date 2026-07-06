//! Agrégation des sources en un index unique et dédupliqué.

use std::collections::HashSet;

use crate::model::AppEntry;
use crate::sources::AppSource;

pub struct Index {
    entries: Vec<AppEntry>,
}

impl Index {
    /// Construit l'index avec les sources par défaut du système réel.
    pub fn build() -> Self {
        use crate::sources::{desktop::DesktopScanner, steam::SteamScanner};
        Self::from_sources(&[&DesktopScanner::new(), &SteamScanner::new()])
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
        Self { entries }
    }

    pub fn entries(&self) -> &[AppEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
