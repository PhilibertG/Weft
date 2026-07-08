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
pub mod epic;
pub mod icon;
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

/// Options d'installation. `Default` = comportement 2.2 (installeur
/// interactif, pas de correctifs par-jeu).
#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// Id protonfixes (champ gameid du manifest).
    pub gameid: Option<String>,
    /// Store d'origine ("gog", "egs") pour la recherche de correctifs.
    pub store: Option<String>,
    /// Identifiant du jeu chez son store.
    pub store_id: Option<String>,
    /// Nom d'affichage imposé (les stores connaissent le vrai titre —
    /// sinon, celui découvert dans le préfixe).
    pub name: Option<String>,
    /// Installeur exécuté sans assistant (flags silencieux Inno/NSIS/MSI).
    /// Utilisé par les installs pilotées (GOG) où personne ne clique.
    pub silent: bool,
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
    /// préfixe isolé et l'enregistre.
    pub fn install(
        &self,
        file: &Path,
        opts: InstallOptions,
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
                copy_portable(file, &prefix)
            }
            installer::InstallerKind::Msi => {
                progress(if opts.silent {
                    "Installation en cours…"
                } else {
                    "Installation (Windows Installer)… suis l'assistant s'il s'affiche."
                });
                let after: &[&str] = if opts.silent { &["/qn"] } else { &[] };
                self.run_in_prefix_blocking(
                    &prefix,
                    &install_log,
                    &["msiexec", "/i"],
                    Some(file),
                    after,
                )
            }
            installer::InstallerKind::Inno => {
                progress(if opts.silent {
                    "Installation en cours…"
                } else {
                    "Installation… suis l'assistant qui va s'afficher."
                });
                let after: &[&str] = if opts.silent {
                    &["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]
                } else {
                    &[]
                };
                self.run_in_prefix_blocking(&prefix, &install_log, &[], Some(file), after)
            }
            installer::InstallerKind::Nsis => {
                progress(if opts.silent {
                    "Installation en cours…"
                } else {
                    "Installation… suis l'assistant qui va s'afficher."
                });
                let after: &[&str] = if opts.silent { &["/S"] } else { &[] };
                self.run_in_prefix_blocking(&prefix, &install_log, &[], Some(file), after)
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
            name: opts.name.unwrap_or_else(|| main.name.clone()),
            exe: main.exe.clone(),
            gameid: opts.gameid,
            store: opts.store,
            store_id: opts.store_id,
            created: now_rfc3339(),
            runtime: RuntimeVersions {
                proton: runtime::PINNED_PROTON.to_owned(),
                umu: runtime::PINNED_UMU.to_owned(),
            },
        };

        // Réinstallation du même programme (même nom, même exe) : on
        // REMPLACE l'app existante au lieu d'empiler un doublon -2, en
        // reprenant son slug — l'identité (et donc la frecency) survit.
        let (slug, dir) = match self.find_same_app(&slug, &manifest) {
            Some(old_slug) => {
                progress(&format!("« {} » déjà installé : remplacement.", manifest.name));
                self.store.remove(&old_slug)?;
                let old_dir = dir.parent().unwrap().join(&old_slug);
                std::fs::rename(&dir, &old_dir)?;
                (old_slug, old_dir)
            }
            None => (slug, dir),
        };

        manifest.save(&dir.join("manifest.toml"))?;

        // Icône de l'exe (best-effort, jamais bloquant). `dir` a pu être
        // renommé par le remplacement : on repart de lui.
        icon::extract_icon(
            &dir.join("prefix").join(&manifest.exe),
            &dir.join("icon.png"),
        );

        progress(&format!("« {} » installé.", manifest.name));

        Ok(InstalledApp { slug, dir, manifest })
    }

    /// Installe un jeu Epic via legendary : téléchargement dans
    /// `apps/<slug>/game/`, préfixe isolé, gameid protonfixes depuis la
    /// base umu (best-effort), manifest store=egs.
    pub fn install_epic(
        &self,
        app_name: &str,
        mut progress: impl FnMut(&str),
    ) -> io::Result<InstalledApp> {
        if !self.runtime.status().ready() {
            return Err(io::Error::other(
                "environnement Windows pas encore prêt (weft-windows runtime fetch)",
            ));
        }
        if !epic::available() {
            return Err(io::Error::other(
                "support Epic non installé (outil legendary manquant)",
            ));
        }
        if !epic::logged_in() {
            return Err(io::Error::other(
                "aucun compte Epic connecté (weft-windows epic login)",
            ));
        }
        let title = epic::library()?
            .into_iter()
            .find(|g| g.app_name == app_name)
            .ok_or_else(|| {
                io::Error::other(format!("« {app_name} » n'est pas dans ta bibliothèque Epic"))
            })?
            .title;

        // Réinstallation : même jeu déjà là => on repart de son slug.
        let existing = self
            .store
            .list()
            .into_iter()
            .find(|a| a.manifest.store_id.as_deref() == Some(app_name));
        let (slug, dir) = match existing {
            Some(app) => {
                progress(&format!("« {} » déjà installé : remplacement.", app.manifest.name));
                self.store.remove(&app.slug)?;
                self.store.create(&title)?
            }
            None => self.store.create(&title)?,
        };

        progress(&format!("Téléchargement de « {title} »…"));
        let log = std::fs::File::create(dir.join("logs/install.log"))?;
        let status = Command::new("legendary")
            .args(["install", app_name, "-y", "--base-path"])
            .arg(dir.join("game"))
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .status()?;
        if !status.success() {
            let _ = self.store.remove(&slug);
            return Err(io::Error::other("le téléchargement a échoué (connexion ?)"));
        }

        // Exe principal déclaré par Epic, exprimé relatif au dossier d'app.
        let Some((install_path, exe)) = epic::installed_info(app_name) else {
            let _ = self.store.remove(&slug);
            return Err(io::Error::other("jeu téléchargé mais introuvable (legendary)"));
        };
        let exe_rel = match install_path.strip_prefix(&dir) {
            Ok(rel) => format!("{}/{exe}", rel.display()),
            Err(_) => format!("{}/{exe}", install_path.display()), // hors app : absolu
        };

        progress("Recherche de correctifs connus…");
        let gameid = epic::umu_id(app_name, "egs");

        let manifest = Manifest {
            name: title,
            exe: exe_rel,
            gameid,
            store: Some("egs".to_owned()),
            store_id: Some(app_name.to_owned()),
            created: now_rfc3339(),
            runtime: RuntimeVersions {
                proton: runtime::PINNED_PROTON.to_owned(),
                umu: runtime::PINNED_UMU.to_owned(),
            },
        };
        manifest.save(&dir.join("manifest.toml"))?;

        let app = InstalledApp { slug, dir, manifest };
        icon::extract_icon(&app.exe_path(), &app.dir.join("icon.png"));
        progress(&format!("« {} » installé.", app.manifest.name));
        Ok(app)
    }

    /// Une app déjà installée qui est "le même programme" que celui qu'on
    /// vient d'installer sous `current_slug` : même nom ET même exe.
    fn find_same_app(&self, current_slug: &str, manifest: &Manifest) -> Option<String> {
        self.store
            .list()
            .into_iter()
            .find(|a| {
                a.slug != current_slug
                    && a.manifest.name == manifest.name
                    && a.manifest.exe == manifest.exe
            })
            .map(|a| a.slug)
    }

    /// (Ré)extrait les icônes de toutes les apps installées. Retourne les
    /// slugs pour lesquels une icône a été produite.
    pub fn refresh_icons(&self) -> Vec<String> {
        self.store
            .list()
            .into_iter()
            .filter(|app| icon::extract_icon(&app.exe_path(), &app.dir.join("icon.png")))
            .map(|app| app.slug)
            .collect()
    }

    /// Lance une app installée, détachée, logs dans logs/launch.log.
    ///
    /// Jeux Epic : lancés PAR legendary (il injecte les arguments
    /// d'authentification Epic Online Services) avec umu-run en wrapper —
    /// même préfixe, même runtime épinglé, mêmes protonfixes.
    pub fn launch(&self, slug: &str) -> io::Result<()> {
        let app = self
            .store
            .get(slug)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("app inconnue : {slug}")))?;

        let is_epic = app.manifest.store_or_none() == "egs" && app.manifest.store_id.is_some();
        let mut cmd = if is_epic && epic::available() {
            let mut c = Command::new("legendary");
            c.args([
                "launch",
                app.manifest.store_id.as_deref().unwrap(),
                "--no-wine",
                "--wrapper",
            ])
            .arg(self.runtime.umu_run());
            self.apply_umu_env(&mut c, &app.prefix_dir(), &app.manifest);
            c
        } else {
            let mut c = self.umu_command(&app.prefix_dir(), &app.manifest);
            c.arg(app.exe_path());
            c
        };

        let log = std::fs::File::create(app.logs_dir().join("launch.log"))?;
        cmd.stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd.spawn()?;
        Ok(())
    }

    /// Commande umu-run configurée pour un préfixe.
    fn umu_command(&self, prefix: &Path, manifest: &Manifest) -> Command {
        let mut cmd = Command::new(self.runtime.umu_run());
        self.apply_umu_env(&mut cmd, prefix, manifest);
        cmd
    }

    /// Variables d'environnement umu : versions épinglées du manifest si
    /// présentes sur disque, sinon celles de Weft.
    fn apply_umu_env(&self, cmd: &mut Command, prefix: &Path, manifest: &Manifest) {
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
        cmd.env("WINEPREFIX", prefix)
            .env("PROTONPATH", proton)
            .env("GAMEID", manifest.gameid_or_default())
            // Le store d'origine route la recherche protonfixes
            // (gamefixes-gog, gamefixes-egs...). "none" pour les autres.
            .env("STORE", manifest.store_or_none());
    }

    /// Exécute une commande dans le préfixe (installeur...), bloquant,
    /// sortie vers un fichier de log jamais montré à l'utilisateur.
    fn run_in_prefix_blocking(
        &self,
        prefix: &Path,
        log_path: &Path,
        args: &[&str],
        file: Option<&Path>,
        args_after: &[&str],
    ) -> io::Result<()> {
        let log = std::fs::File::create(log_path)?;
        let mut cmd = Command::new(self.runtime.umu_run());
        cmd.args(args);
        if let Some(f) = file {
            cmd.arg(f);
        }
        cmd.args(args_after);
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

/// Copie un programme portable dans le préfixe.
///
/// Un exe "portable" est rarement seul : jeux Electron/Unity, outils avec
/// DLLs — l'exe a besoin de ses fichiers voisins (PolyTrack sans son
/// icudtl.dat crashe immédiatement). Si le dossier parent ressemble à un
/// dossier d'application, on le copie ENTIER ; sinon (exe posé dans
/// Téléchargements au milieu d'autres fichiers), l'exe seul.
fn copy_portable(file: &Path, prefix: &Path) -> io::Result<()> {
    let dest_root = prefix.join("drive_c/weft-portable");
    match file.parent().filter(|p| is_app_folder(p, file)) {
        Some(parent) => {
            let dir_name = parent.file_name().unwrap_or_default();
            copy_dir_recursive(parent, &dest_root.join(dir_name))
        }
        None => {
            std::fs::create_dir_all(&dest_root)?;
            std::fs::copy(file, dest_root.join(file.file_name().unwrap_or_default()))
                .map(|_| ())
        }
    }
}

/// Le dossier parent est-il LE dossier de l'application, ou juste un
/// endroit où l'exe traîne ? Deux gardes :
/// - jamais un répertoire "fourre-tout" (home, Téléchargements, Bureau...) ;
/// - l'exe choisi doit être le seul .exe de premier niveau (deux exes =>
///   probablement des programmes sans rapport, on ne prend pas le risque).
fn is_app_folder(dir: &Path, exe: &Path) -> bool {
    if is_common_dir(dir) {
        return false;
    }
    let Ok(read) = std::fs::read_dir(dir) else { return false };
    let mut entries = 0usize;
    for e in read.flatten() {
        let p = e.path();
        entries += 1;
        let is_exe = p
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("exe"));
        if is_exe && p != exe {
            return false;
        }
    }
    entries > 1 // l'exe seul dans un dossier : rien d'autre à copier
}

/// Répertoires standards où un exe téléchargé atterrit : on ne copie
/// JAMAIS tout leur contenu.
fn is_common_dir(dir: &Path) -> bool {
    let home = std::env::var("HOME").map(PathBuf::from).ok();
    if home.as_deref() == Some(dir) {
        return true;
    }
    if dir == Path::new("/tmp") {
        return true;
    }
    const COMMON: &[&str] = &[
        "Téléchargements", "Downloads", "Bureau", "Desktop", "Documents",
        "Images", "Pictures", "Videos", "Vidéos", "Musique", "Music",
    ];
    home.is_some_and(|h| COMMON.iter().any(|n| h.join(n) == dir))
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for e in std::fs::read_dir(src)?.flatten() {
        let from = e.path();
        let to = dest.join(e.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("weft-portable-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn app_folder_with_data_files_is_copied_whole() {
        let root = temp("appdir");
        let app = root.join("MonJeu-v1.0");
        std::fs::create_dir_all(app.join("resources")).unwrap();
        std::fs::write(app.join("jeu.exe"), b"MZ").unwrap();
        std::fs::write(app.join("icudtl.dat"), b"data").unwrap();
        std::fs::write(app.join("resources/app.pak"), b"pak").unwrap();

        let prefix = root.join("prefix");
        copy_portable(&app.join("jeu.exe"), &prefix).unwrap();

        let copied = prefix.join("drive_c/weft-portable/MonJeu-v1.0");
        assert!(copied.join("jeu.exe").is_file());
        assert!(copied.join("icudtl.dat").is_file());
        assert!(copied.join("resources/app.pak").is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn folder_with_several_exes_copies_only_the_chosen_one() {
        let root = temp("multi");
        std::fs::write(root.join("a.exe"), b"MZ").unwrap();
        std::fs::write(root.join("b.exe"), b"MZ").unwrap();
        std::fs::write(root.join("notes.txt"), b"x").unwrap();

        let prefix = root.join("prefix");
        copy_portable(&root.join("a.exe"), &prefix).unwrap();

        let dest = prefix.join("drive_c/weft-portable");
        assert!(dest.join("a.exe").is_file());
        assert!(!dest.join("b.exe").exists());
        assert!(!dest.join("notes.txt").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn common_dirs_are_never_copied_whole() {
        let home = std::env::var("HOME").unwrap();
        assert!(is_common_dir(Path::new(&home)));
        assert!(is_common_dir(&Path::new(&home).join("Téléchargements")));
        assert!(is_common_dir(&Path::new(&home).join("Downloads")));
        assert!(is_common_dir(Path::new("/tmp")));
        assert!(!is_common_dir(&Path::new(&home).join("Downloads/PolyTrack-v0.6.2")));
    }
}
