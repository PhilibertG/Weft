//! Configuration utilisateur : ~/.config/weft/config.toml.
//!
//! Créé avec des valeurs par défaut commentées au premier lancement.
//! Une config illisible ne casse jamais le launcher : on râle sur stderr
//! et on repart sur les défauts. Le raccourci clavier n'est PAS ici : en
//! Wayland il appartient au bureau (réglages GNOME).

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Config {
    pub window: WindowConfig,
    pub providers: ProvidersConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowConfig {
    pub width: i32,
    pub height: i32,
    /// Nombre max de résultats affichés quand on tape (la requête vide
    /// liste tout).
    pub max_results: usize,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 620,
            height: 440,
            max_results: 8,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProvidersConfig {
    pub apps: bool,
    pub calc: bool,
    pub files: bool,
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        Self {
            apps: true,
            calc: true,
            files: true,
        }
    }
}

const DEFAULT_FILE: &str = "\
# Configuration de Weft (~/.config/weft/config.toml)
# Le raccourci clavier se règle dans les paramètres GNOME
# (Clavier > Raccourcis personnalisés), pas ici.

[window]
width = 620
height = 440
# Nombre max de résultats affichés pendant la frappe.
max_results = 8

[providers]
apps = true   # applications (natives, Flatpak, Steam, Wine)
calc = true   # calculatrice inline (23*7, 100 km en miles...)
files = true  # recherche de fichiers (nécessite plocate)
";

pub fn config_path() -> Option<PathBuf> {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|d| d.join("weft/config.toml"))
        .ok()
}

impl Config {
    /// Charge la config, en créant le fichier par défaut s'il n'existe pas.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(_) => {
                // Premier lancement : matérialiser les défauts, commentés,
                // pour que la config soit découvrable.
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&path, DEFAULT_FILE);
                Self::default()
            }
        }
    }

    pub fn parse(text: &str) -> Self {
        toml::from_str(text).unwrap_or_else(|e| {
            eprintln!("weft: config.toml illisible ({e}), défauts utilisés");
            Self::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_parses_to_defaults() {
        assert_eq!(Config::parse(DEFAULT_FILE), Config::default());
    }

    #[test]
    fn partial_config_keeps_other_defaults() {
        let cfg = Config::parse("[providers]\nfiles = false\n");
        assert!(!cfg.providers.files);
        assert!(cfg.providers.apps);
        assert_eq!(cfg.window.width, 620);
    }

    #[test]
    fn broken_config_falls_back_to_defaults() {
        assert_eq!(Config::parse("ceci n'est {{ pas du toml"), Config::default());
    }
}
