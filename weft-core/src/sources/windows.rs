//! AppSource "weft-windows" : les programmes Windows installés par Weft,
//! exposés comme n'importe quelle app. C'est le critère de succès de la
//! brique 2.1 : dans le launcher, elles sont indistinguables des natives.

use crate::model::{AppEntry, Icon, LaunchSpec, Source};
use crate::sources::AppSource;
use crate::windows::prefix::AppStore;
use crate::windows::WindowsRoot;

pub struct WindowsAppsScanner {
    store: AppStore,
}

impl WindowsAppsScanner {
    pub fn new() -> Option<Self> {
        WindowsRoot::open_default().map(|root| Self {
            store: AppStore::new(root),
        })
    }

    /// Racine explicite (tests).
    pub fn with_root(root: WindowsRoot) -> Self {
        Self {
            store: AppStore::new(root),
        }
    }
}

impl AppSource for WindowsAppsScanner {
    fn name(&self) -> &'static str {
        "weft-windows"
    }

    fn scan(&self) -> Vec<AppEntry> {
        self.store
            .list()
            .into_iter()
            .map(|app| AppEntry {
                id: format!("weft-windows:{}", app.slug),
                name: app.manifest.name.clone(),
                description: None,
                // Icône extraite de l'exe à l'installation ; fallback UI
                // si l'extraction n'a rien donné.
                icon: app.icon_path().map(Icon::Path),
                launch: LaunchSpec::WindowsApp(app.slug),
                source: Source::Wine,
                keywords: Vec::new(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows::manifest::{Manifest, RuntimeVersions};

    #[test]
    fn scans_manifests_into_entries() {
        let root = WindowsRoot::at(
            std::env::temp_dir().join(format!("weft-src-win-{}", std::process::id())),
        );
        let _ = std::fs::remove_dir_all(root.path());
        let store = AppStore::new(root.clone());
        let (slug, dir) = store.create("WinMerge").unwrap();
        Manifest {
            name: "WinMerge".into(),
            exe: "drive_c/Program Files/WinMerge/WinMergeU.exe".into(),
            gameid: None,
            store: None,
            store_id: None,
            created: String::new(),
            runtime: RuntimeVersions { proton: "p".into(), umu: "u".into() },
        }
        .save(&dir.join("manifest.toml"))
        .unwrap();

        let entries = WindowsAppsScanner::with_root(root.clone()).scan();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "WinMerge");
        assert_eq!(entries[0].id, format!("weft-windows:{slug}"));
        assert_eq!(entries[0].launch, LaunchSpec::WindowsApp(slug));

        let _ = std::fs::remove_dir_all(root.path());
    }
}
