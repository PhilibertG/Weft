//! Gestion des apps Windows installées : un répertoire par app sous
//! `apps/<slug>/`, contenant le manifest, le préfixe Wine ISOLÉ et les
//! logs. Pas de préfixe partagé fourre-tout : la casse d'une app n'en
//! contamine jamais une autre.

use std::io;
use std::path::PathBuf;

use super::manifest::{slugify, Manifest};
use super::WindowsRoot;

/// Une app installée : son répertoire et son manifest chargé.
#[derive(Debug, Clone)]
pub struct InstalledApp {
    pub slug: String,
    pub dir: PathBuf,
    pub manifest: Manifest,
}

impl InstalledApp {
    pub fn prefix_dir(&self) -> PathBuf {
        self.dir.join("prefix")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.dir.join("logs")
    }

    /// Chemin absolu de l'exe principal. L'exe vit dans le préfixe
    /// (installeurs classiques) ou dans le répertoire de l'app (jeux des
    /// stores, posés dans `game/` hors du préfixe).
    pub fn exe_path(&self) -> PathBuf {
        let in_prefix = self.prefix_dir().join(&self.manifest.exe);
        if in_prefix.exists() {
            in_prefix
        } else {
            self.dir.join(&self.manifest.exe)
        }
    }

    /// L'icône extraite de l'exe, si l'extraction a réussi un jour.
    pub fn icon_path(&self) -> Option<PathBuf> {
        let p = self.dir.join("icon.png");
        p.is_file().then_some(p)
    }
}

pub struct AppStore {
    root: WindowsRoot,
}

impl AppStore {
    pub fn new(root: WindowsRoot) -> Self {
        Self { root }
    }

    /// Toutes les apps valides. Un manifest illisible est ignoré (avec un
    /// avertissement stderr), jamais bloquant — dégradation propre.
    pub fn list(&self) -> Vec<InstalledApp> {
        let mut apps = Vec::new();
        let Ok(read) = std::fs::read_dir(self.root.apps_dir()) else {
            return apps;
        };
        for e in read.flatten() {
            let dir = e.path();
            if !dir.is_dir() {
                continue;
            }
            // Pas de manifest : installation en cours ou répertoire
            // étranger — on passe sans bruit.
            if !dir.join("manifest.toml").is_file() {
                continue;
            }
            match Manifest::load(&dir.join("manifest.toml")) {
                Ok(manifest) => apps.push(InstalledApp {
                    slug: dir.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                    dir,
                    manifest,
                }),
                Err(e) => eprintln!("weft: app ignorée ({e})"),
            }
        }
        apps.sort_by(|a, b| a.slug.cmp(&b.slug));
        apps
    }

    pub fn get(&self, slug: &str) -> Option<InstalledApp> {
        self.list().into_iter().find(|a| a.slug == slug)
    }

    /// Réserve un répertoire d'app + préfixe vierge. Le slug est dérivé du
    /// nom, suffixé -2, -3... en cas de collision (deux installs du même
    /// programme restent deux apps isolées).
    pub fn create(&self, name: &str) -> io::Result<(String, PathBuf)> {
        let base = slugify(name);
        let apps = self.root.apps_dir();
        let mut slug = base.clone();
        let mut n = 1;
        while apps.join(&slug).exists() {
            n += 1;
            slug = format!("{base}-{n}");
        }
        let dir = apps.join(&slug);
        std::fs::create_dir_all(dir.join("prefix"))?;
        std::fs::create_dir_all(dir.join("logs"))?;
        Ok((slug, dir))
    }

    /// Suppression complète : préfixe, logs, manifest. Irréversible et
    /// assumé (c'est le "désinstaller" de Weft).
    pub fn remove(&self, slug: &str) -> io::Result<()> {
        let dir = self.root.apps_dir().join(slug);
        if !dir.join("manifest.toml").is_file() && !dir.join("prefix").is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("app inconnue : {slug}"),
            ));
        }
        std::fs::remove_dir_all(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows::manifest::RuntimeVersions;

    fn store(tag: &str) -> AppStore {
        let p = std::env::temp_dir().join(format!("weft-apps-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        AppStore::new(WindowsRoot::at(p))
    }

    fn write_manifest(dir: &std::path::Path, name: &str) {
        Manifest {
            name: name.into(),
            exe: "drive_c/app.exe".into(),
            gameid: None,
            store: None,
            store_id: None,
            created: String::new(),
            runtime: RuntimeVersions { proton: "p".into(), umu: "u".into() },
        }
        .save(&dir.join("manifest.toml"))
        .unwrap();
    }

    #[test]
    fn create_list_remove_lifecycle() {
        let s = store("lifecycle");

        let (slug, dir) = s.create("WinMerge").unwrap();
        assert_eq!(slug, "winmerge");
        assert!(dir.join("prefix").is_dir());
        write_manifest(&dir, "WinMerge");

        assert_eq!(s.list().len(), 1);
        assert_eq!(s.get("winmerge").unwrap().manifest.name, "WinMerge");

        s.remove("winmerge").unwrap();
        assert!(s.list().is_empty());
        assert!(s.remove("winmerge").is_err()); // déjà supprimée
    }

    #[test]
    fn slug_collisions_get_suffixed() {
        let s = store("collision");
        let (a, dir_a) = s.create("App").unwrap();
        write_manifest(&dir_a, "App");
        let (b, _) = s.create("App").unwrap();
        assert_eq!(a, "app");
        assert_eq!(b, "app-2");
    }

    #[test]
    fn broken_manifest_is_skipped_not_fatal() {
        let s = store("broken");
        let (_, dir) = s.create("Bonne App").unwrap();
        write_manifest(&dir, "Bonne App");
        let (_, dir_bad) = s.create("Cassée").unwrap();
        std::fs::write(dir_bad.join("manifest.toml"), "pas du toml {{").unwrap();

        let apps = s.list();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].manifest.name, "Bonne App");
    }
}
