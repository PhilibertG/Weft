//! Pilotage du runtime umu/Proton, versions épinglées.
//!
//! Weft télécharge lui-même les composants aux versions exactes ci-dessous
//! et passe toujours `PROTONPATH` explicite à umu-run : umu ne choisit
//! jamais une version. Le conteneur Steam Linux Runtime est la seule pièce
//! gérée par umu (dans ~/.local/share/umu) ; sa version découle de celle
//! d'umu, épinglée ici.
//!
//! Téléchargements via `curl` système (pas de stack HTTP embarquée dans
//! weft-core), vérification sha512 pour Proton.

use std::io;
use std::path::PathBuf;
use std::process::Command;

use super::WindowsRoot;

/// Versions épinglées par Weft. Monter de version = changer ces constantes
/// (et, plus tard, les métadonnées des apps qui migrent).
pub const PINNED_UMU: &str = "1.4.1";
pub const PINNED_PROTON: &str = "UMU-Proton-10.0-4";

const UMU_URL: &str = "https://github.com/Open-Wine-Components/umu-launcher/releases/download";
const PROTON_URL: &str = "https://github.com/Open-Wine-Components/umu-proton/releases/download";

/// État du runtime, exposé pour l'UX (2.2) : chaque pièce est détectable,
/// le téléchargement est déclenchable explicitement via `fetch()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStatus {
    /// umu-run épinglé présent ?
    pub umu: bool,
    /// UMU-Proton épinglé présent ?
    pub proton: bool,
    /// Conteneur Steam Linux Runtime déjà téléchargé par umu ?
    pub container: bool,
    /// python3 sur la machine (nécessaire au zipapp umu) ?
    pub python: bool,
}

impl RuntimeStatus {
    pub fn ready(&self) -> bool {
        self.umu && self.proton && self.python
        // container pas exigé : umu le télécharge au premier lancement,
        // fetch_container() permet de le faire en avance de phase.
    }
}

pub struct Runtime {
    root: WindowsRoot,
}

impl Runtime {
    pub fn new(root: WindowsRoot) -> Self {
        Self { root }
    }

    /// Chemin de l'exécutable umu-run épinglé.
    pub fn umu_run(&self) -> PathBuf {
        self.root
            .runtimes_dir()
            .join(format!("umu/{PINNED_UMU}/umu-run"))
    }

    /// Répertoire Proton épinglé (valeur de PROTONPATH).
    pub fn proton_dir(&self) -> PathBuf {
        self.root
            .runtimes_dir()
            .join(format!("proton/{PINNED_PROTON}"))
    }

    /// Répertoire du conteneur géré par umu.
    fn umu_container_dir() -> Option<PathBuf> {
        std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|d| d.join("umu"))
            .ok()
    }

    pub fn status(&self) -> RuntimeStatus {
        RuntimeStatus {
            umu: self.umu_run().is_file(),
            proton: self.proton_dir().join("proton").is_file(),
            container: Self::umu_container_dir()
                .is_some_and(|d| d.is_dir() && std::fs::read_dir(&d).map(|mut r| r.next().is_some()).unwrap_or(false)),
            python: which("python3"),
        }
    }

    /// Télécharge les composants épinglés manquants (umu + Proton).
    /// `progress` reçoit des messages humains (affichés par le CLI en 2.1,
    /// par l'UI en 2.2).
    pub fn fetch(&self, mut progress: impl FnMut(&str)) -> io::Result<()> {
        let status = self.status();
        if !status.python {
            return Err(other("python3 introuvable (requis par umu)"));
        }

        if !status.umu {
            progress(&format!("Téléchargement d'umu {PINNED_UMU}…"));
            self.fetch_umu()?;
            progress("umu installé.");
        }
        if !status.proton {
            progress(&format!("Téléchargement de {PINNED_PROTON} (~490 Mo)…"));
            self.fetch_proton(&mut progress)?;
            progress("Proton installé et vérifié.");
        }
        Ok(())
    }

    /// Déclenche le téléchargement du conteneur par umu (premier lancement
    /// à vide dans un préfixe jetable). Long (~500 Mo) ; explicite exprès.
    pub fn fetch_container(&self, mut progress: impl FnMut(&str)) -> io::Result<()> {
        if !self.status().ready() {
            return Err(other("runtime incomplet : lancer fetch() d'abord"));
        }
        progress("Initialisation du conteneur Steam Linux Runtime (long au premier coup)…");
        let scratch = self.root.path().join("tmp/container-warmup");
        std::fs::create_dir_all(&scratch)?;
        let out = Command::new(self.umu_run())
            .arg("createprefix")
            .env("WINEPREFIX", &scratch)
            .env("PROTONPATH", self.proton_dir())
            .env("GAMEID", "umu-default")
            .output()?;
        let _ = std::fs::remove_dir_all(self.root.path().join("tmp"));
        if !out.status.success() && !self.status().container {
            return Err(other(&format!(
                "échec d'initialisation du conteneur : {}",
                String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("?")
            )));
        }
        progress("Conteneur prêt.");
        Ok(())
    }

    fn fetch_umu(&self) -> io::Result<()> {
        let dir = self.umu_run().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let url = format!("{UMU_URL}/{PINNED_UMU}/umu-launcher-{PINNED_UMU}-zipapp.tar");
        let tmp = self.tmp_dir()?;
        let tarball = tmp.join("umu.tar");
        download(&url, &tarball)?;
        run_in(&tmp, "tar", &["-xf", "umu.tar"])?;
        // Le tar contient umu-run (zipapp), potentiellement dans un
        // sous-répertoire : on le cherche.
        let found = find_file(&tmp, "umu-run")
            .ok_or_else(|| other("umu-run absent de l'archive zipapp"))?;
        std::fs::copy(&found, self.umu_run())?;
        make_executable(&self.umu_run())?;
        let _ = std::fs::remove_dir_all(&tmp);
        Ok(())
    }

    fn fetch_proton(&self, progress: &mut impl FnMut(&str)) -> io::Result<()> {
        let parent = self.proton_dir().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&parent)?;
        let tmp = self.tmp_dir()?;
        let tarball = tmp.join(format!("{PINNED_PROTON}.tar.gz"));
        let sums = tmp.join(format!("{PINNED_PROTON}.sha512sum"));

        download(
            &format!("{PROTON_URL}/{PINNED_PROTON}/{PINNED_PROTON}.tar.gz"),
            &tarball,
        )?;
        download(
            &format!("{PROTON_URL}/{PINNED_PROTON}/{PINNED_PROTON}.sha512sum"),
            &sums,
        )?;

        progress("Vérification sha512…");
        run_in(&tmp, "sha512sum", &["-c", &sums.file_name_str()])?;

        progress("Extraction…");
        run_in(&tmp, "tar", &["-xzf", &tarball.file_name_str()])?;
        let extracted = tmp.join(PINNED_PROTON);
        if !extracted.join("proton").is_file() {
            return Err(other("archive Proton inattendue (script proton absent)"));
        }
        // rename échoue entre systèmes de fichiers différents : tmp est
        // volontairement sous la même racine que la destination.
        std::fs::rename(&extracted, self.proton_dir())?;
        let _ = std::fs::remove_dir_all(&tmp);
        Ok(())
    }

    fn tmp_dir(&self) -> io::Result<PathBuf> {
        let dir = self.root.path().join("tmp/fetch");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

fn which(bin: &str) -> bool {
    std::env::var("PATH").is_ok_and(|path| {
        path.split(':')
            .any(|d| PathBuf::from(d).join(bin).is_file())
    })
}

fn download(url: &str, dest: &std::path::Path) -> io::Result<()> {
    let status = Command::new("curl")
        .args(["-L", "--fail", "--silent", "--show-error", "-o"])
        .arg(dest)
        .arg(url)
        .status()?;
    if !status.success() {
        return Err(other(&format!("téléchargement échoué : {url}")));
    }
    Ok(())
}

fn run_in(dir: &std::path::Path, bin: &str, args: &[&str]) -> io::Result<()> {
    let out = Command::new(bin).args(args).current_dir(dir).output()?;
    if !out.status.success() {
        return Err(other(&format!(
            "{bin} a échoué : {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

fn find_file(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| n == name) {
                return Some(p);
            }
        }
    }
    None
}

fn make_executable(path: &std::path::Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
}

fn other(msg: &str) -> io::Error {
    io::Error::other(msg.to_owned())
}

trait FileNameStr {
    fn file_name_str(&self) -> String;
}

impl FileNameStr for PathBuf {
    fn file_name_str(&self) -> String {
        self.file_name().unwrap_or_default().to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> WindowsRoot {
        let p = std::env::temp_dir().join(format!("weft-win-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        WindowsRoot::at(p)
    }

    #[test]
    fn status_on_empty_root_reports_missing() {
        let rt = Runtime::new(temp_root("empty"));
        let s = rt.status();
        assert!(!s.umu);
        assert!(!s.proton);
        assert!(!s.ready());
    }

    #[test]
    fn status_detects_pinned_components() {
        let root = temp_root("present");
        let rt = Runtime::new(root.clone());

        std::fs::create_dir_all(rt.umu_run().parent().unwrap()).unwrap();
        std::fs::write(rt.umu_run(), "#!stub").unwrap();
        std::fs::create_dir_all(rt.proton_dir()).unwrap();
        std::fs::write(rt.proton_dir().join("proton"), "#!stub").unwrap();

        let s = rt.status();
        assert!(s.umu);
        assert!(s.proton);
        // python présent sur toute machine de dev/test raisonnable.
        assert!(s.ready());

        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn pinned_paths_contain_versions() {
        let rt = Runtime::new(temp_root("paths"));
        assert!(rt.umu_run().to_string_lossy().contains(PINNED_UMU));
        assert!(rt.proton_dir().to_string_lossy().contains(PINNED_PROTON));
    }
}
