//! Moteur Windows de Weft (brique 2) : installer et lancer des programmes
//! Windows arbitraires via umu/Proton, sans que l'utilisateur ne voie
//! jamais Wine, Proton ou un préfixe.
//!
//! Décision d'architecture (étude 2.1) : runtime = **umu-launcher**.
//! Proton + protonfixes hors Steam, bibliothèques dans le conteneur Steam
//! Linux Runtime => aucune dépendance i386 sur l'hôte (critère décisif pour
//! l'OS final). Versions TOUJOURS épinglées par Weft, jamais de latest
//! implicite.

pub mod manifest;
pub mod prefix;
pub mod runtime;

use std::path::{Path, PathBuf};

/// Racine de tout le monde Windows de Weft
/// (`~/.local/share/weft/windows/`). Paramétrable pour les tests.
#[derive(Debug, Clone)]
pub struct WindowsRoot(PathBuf);

impl WindowsRoot {
    pub fn open_default() -> Option<Self> {
        std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|d| Self(d.join("weft/windows")))
            .ok()
    }

    pub fn at(path: PathBuf) -> Self {
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn runtimes_dir(&self) -> PathBuf {
        self.0.join("runtimes")
    }

    pub fn apps_dir(&self) -> PathBuf {
        self.0.join("apps")
    }
}
