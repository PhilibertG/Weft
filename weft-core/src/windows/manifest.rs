//! Métadonnées d'une app Windows installée par Weft : manifest.toml à la
//! racine du répertoire de l'app. C'est la source de vérité que lira
//! l'AppSource "weft-windows" — et l'OS final plus tard.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Nom affiché ("WinMerge"), jamais de jargon.
    pub name: String,
    /// Exe principal, relatif au préfixe ("drive_c/Program Files/...").
    pub exe: String,
    /// Id protonfixes ; None => "umu-default" (pas de correctifs par-jeu).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gameid: Option<String>,
    /// Store d'origine pour la recherche de correctifs ("gog", "egs") ;
    /// None => "none". Jamais montré dans l'UI, comme le reste.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    /// Identifiant du jeu chez son store (app_name Epic, slug GOG) —
    /// nécessaire pour lancer/mettre à jour via l'outil du store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_id: Option<String>,
    /// Date d'installation, RFC 3339.
    pub created: String,
    /// Versions épinglées au moment de l'installation.
    pub runtime: RuntimeVersions,
    /// Variables d'environnement par-app au lancement (ex.
    /// PROTON_USE_WINED3D=1 posé automatiquement pour les jeux 32 bits
    /// quand l'hôte n'a pas de Vulkan i386). Jamais montré à l'utilisateur.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeVersions {
    pub proton: String,
    pub umu: String,
}

impl Manifest {
    pub fn gameid_or_default(&self) -> &str {
        self.gameid.as_deref().unwrap_or("umu-default")
    }

    pub fn store_or_none(&self) -> &str {
        self.store.as_deref().unwrap_or("none")
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        toml::from_str(&text).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("manifest illisible {} : {e}", path.display()),
            )
        })
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = toml::to_string_pretty(self)
            .map_err(|e| io::Error::other(format!("sérialisation manifest : {e}")))?;
        // Écriture atomique, même politique que le reste de Weft.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }
}

/// Slug d'app : identifiant stable et sûr pour un nom de répertoire.
pub fn slugify(name: &str) -> String {
    let mut slug: String = name
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-').to_owned();
    if slug.is_empty() {
        "app".to_owned()
    } else {
        slug
    }
}

pub fn now_rfc3339() -> String {
    // Pas de dépendance chrono pour une date : date système.
    std::process::Command::new("date")
        .arg("--rfc-3339=seconds")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

#[allow(unused)]
pub fn manifest_path(app_dir: &Path) -> PathBuf {
    app_dir.join("manifest.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_gameid_default() {
        let m = Manifest {
            name: "WinMerge".into(),
            exe: "drive_c/Program Files/WinMerge/WinMergeU.exe".into(),
            gameid: None,
            store: None,
            store_id: None,
            created: "2026-07-07T18:00:00+02:00".into(),
            runtime: RuntimeVersions {
                proton: "UMU-Proton-10.0-4".into(),
                umu: "1.4.1".into(),
            },
            env: Default::default(),
        };
        let dir = std::env::temp_dir().join(format!("weft-manifest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("manifest.toml");

        m.save(&path).unwrap();
        let loaded = Manifest::load(&path).unwrap();
        assert_eq!(m, loaded);
        assert_eq!(loaded.gameid_or_default(), "umu-default");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn explicit_gameid_wins() {
        let m = Manifest {
            name: "x".into(),
            exe: "x.exe".into(),
            gameid: Some("umu-244210".into()),
            store: None,
            store_id: None,
            created: String::new(),
            runtime: RuntimeVersions { proton: "p".into(), umu: "u".into() },
            env: Default::default(),
        };
        assert_eq!(m.gameid_or_default(), "umu-244210");
    }

    #[test]
    fn slugify_makes_safe_dir_names() {
        assert_eq!(slugify("WinMerge"), "winmerge");
        assert_eq!(slugify("Notepad++ (x64)"), "notepad-x64");
        assert_eq!(slugify("  Éléphant 2000!  "), "éléphant-2000");
        assert_eq!(slugify("///"), "app");
    }
}
