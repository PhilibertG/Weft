//! Moteur Windows de Weft (brique 2) : installer et lancer des programmes
//! Windows arbitraires via umu/Proton, sans que l'utilisateur ne voie
//! jamais Wine, Proton ou un préfixe.
//!
//! Décision d'architecture (étude 2.1) : runtime = **umu-launcher**.
//! Proton + protonfixes hors Steam, bibliothèques dans le conteneur Steam
//! Linux Runtime => aucune dépendance i386 sur l'hôte (critère décisif pour
//! l'OS final). Versions TOUJOURS épinglées par Weft, jamais de latest
//! implicite.

pub mod discover;
pub mod installer;
pub mod manifest;
pub mod prefix;
pub mod runtime;

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use manifest::{now_rfc3339, Manifest, RuntimeVersions};
use prefix::{AppStore, InstalledApp};
use runtime::Runtime;

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

/// Façade du moteur : installer, lancer, lister, supprimer.
pub struct WindowsEngine {
    runtime: Runtime,
    store: AppStore,
}

impl WindowsEngine {
    pub fn new(root: WindowsRoot) -> Self {
        Self {
            runtime: Runtime::new(root.clone()),
            store: AppStore::new(root),
        }
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub fn store(&self) -> &AppStore {
        &self.store
    }

    /// Installe un programme Windows (installeur ou portable) dans son
    /// préfixe isolé et l'enregistre. `gameid` optionnel (protonfixes).
    pub fn install(
        &self,
        file: &Path,
        gameid: Option<String>,
        mut progress: impl FnMut(&str),
    ) -> io::Result<InstalledApp> {
        if !self.runtime.status().ready() {
            return Err(io::Error::other(
                "environnement Windows pas encore prêt (weft-windows runtime fetch)",
            ));
        }
        if !file.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("fichier introuvable : {}", file.display()),
            ));
        }

        let kind = installer::detect(file)?;
        let hint = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "app".to_owned());

        let (slug, dir) = self.store.create(&hint)?;
        let prefix = dir.join("prefix");
        let install_log = dir.join("logs/install.log");

        let run_result = match kind {
            installer::InstallerKind::Unknown => Err(io::Error::other(
                "ce fichier n'est ni un programme Windows ni un installeur reconnu",
            )),
            installer::InstallerKind::PortableExe => {
                progress("Programme portable : copie dans son environnement…");
                let dest_dir = prefix.join("drive_c/weft-portable");
                std::fs::create_dir_all(&dest_dir)?;
                let dest = dest_dir.join(file.file_name().unwrap_or_default());
                std::fs::copy(file, &dest).map(|_| ())
            }
            installer::InstallerKind::Msi => {
                progress("Installation (Windows Installer)… suis l'assistant s'il s'affiche.");
                self.run_in_prefix_blocking(
                    &prefix,
                    &install_log,
                    &["msiexec", "/i"],
                    Some(file),
                )
            }
            installer::InstallerKind::Inno | installer::InstallerKind::Nsis => {
                progress("Installation… suis l'assistant qui va s'afficher.");
                self.run_in_prefix_blocking(&prefix, &install_log, &[], Some(file))
            }
        };

        if let Err(e) = run_result {
            let _ = self.store.remove(&slug);
            return Err(e);
        }

        progress("Recherche du programme installé…");
        let found = discover::discover(&prefix);
        let Some(main) = found.first() else {
            let _ = self.store.remove(&slug);
            return Err(io::Error::other(
                "l'installation n'a laissé aucun programme reconnaissable (annulée ?)",
            ));
        };

        let manifest = Manifest {
            name: main.name.clone(),
            exe: main.exe.clone(),
            gameid,
            created: now_rfc3339(),
            runtime: RuntimeVersions {
                proton: runtime::PINNED_PROTON.to_owned(),
                umu: runtime::PINNED_UMU.to_owned(),
            },
        };
        manifest.save(&dir.join("manifest.toml"))?;
        progress(&format!("« {} » installé.", manifest.name));

        Ok(InstalledApp { slug, dir, manifest })
    }

    /// Lance une app installée, détachée, logs dans logs/launch.log.
    pub fn launch(&self, slug: &str) -> io::Result<()> {
        let app = self
            .store
            .get(slug)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("app inconnue : {slug}")))?;

        let log = std::fs::File::create(app.logs_dir().join("launch.log"))?;
        let mut cmd = self.umu_command(&app.prefix_dir(), &app.manifest)?;
        cmd.arg(app.exe_path())
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd.spawn()?;
        Ok(())
    }

    /// Commande umu-run configurée pour un préfixe : versions épinglées du
    /// manifest si présentes sur disque, sinon celles de Weft.
    fn umu_command(&self, prefix: &Path, manifest: &Manifest) -> io::Result<Command> {
        let pinned_dir = self
            .runtime
            .proton_dir()
            .parent()
            .unwrap()
            .join(&manifest.runtime.proton);
        let proton = if pinned_dir.join("proton").is_file() {
            pinned_dir
        } else {
            // La version de l'app n'est plus sur disque (ménage, machine
            // neuve) : on retombe sur la version épinglée de Weft.
            self.runtime.proton_dir()
        };
        let mut cmd = Command::new(self.runtime.umu_run());
        cmd.env("WINEPREFIX", prefix)
            .env("PROTONPATH", proton)
            .env("GAMEID", manifest.gameid_or_default())
            .env("STORE", "none");
        Ok(cmd)
    }

    /// Exécute une commande dans le préfixe (installeur...), bloquant,
    /// sortie vers un fichier de log jamais montré à l'utilisateur.
    fn run_in_prefix_blocking(
        &self,
        prefix: &Path,
        log_path: &Path,
        args: &[&str],
        file: Option<&Path>,
    ) -> io::Result<()> {
        let log = std::fs::File::create(log_path)?;
        let mut cmd = Command::new(self.runtime.umu_run());
        cmd.args(args);
        if let Some(f) = file {
            cmd.arg(f);
        }
        let status = cmd
            .env("WINEPREFIX", prefix)
            .env("PROTONPATH", self.runtime.proton_dir())
            .env("GAMEID", "umu-default")
            .env("STORE", "none")
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .status()?;
        if !status.success() {
            return Err(io::Error::other(
                "l'installation ne s'est pas terminée correctement",
            ));
        }
        Ok(())
    }
}
