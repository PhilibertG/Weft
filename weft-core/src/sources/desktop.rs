//! Scanner des fichiers .desktop (apps natives, Flatpak, Wine, raccourcis Steam).
//!
//! Toutes les apps "classiques" de la machine passent par ici : les fichiers
//! .desktop sont le standard freedesktop que GNOME/KDE lisent eux-mêmes.
//! Wine n'est pas un scanner séparé : un raccourci Wine EST un fichier
//! .desktop, on le reconnaît à son contenu et on le classe (champ `source`)
//! au moment du parsing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use freedesktop_desktop_entry::{get_languages_from_env, DesktopEntry};

use crate::model::{AppEntry, Icon, LaunchSpec, Source};
use crate::sources::AppSource;

pub struct DesktopScanner {
    /// Répertoires `applications/`, du plus prioritaire au moins prioritaire
    /// (l'utilisateur override le système, comme le veut la spec).
    dirs: Vec<PathBuf>,
    locales: Vec<String>,
    /// Contenu de XDG_CURRENT_DESKTOP, pour honorer OnlyShowIn/NotShowIn.
    current_desktop: Vec<String>,
}

impl DesktopScanner {
    /// Scanner configuré pour le système réel.
    pub fn new() -> Self {
        Self {
            dirs: default_application_dirs(),
            locales: get_languages_from_env(),
            current_desktop: std::env::var("XDG_CURRENT_DESKTOP")
                .map(|v| v.split(':').map(str::to_owned).collect())
                .unwrap_or_default(),
        }
    }

    /// Scanner pointé sur des répertoires arbitraires (tests/fixtures).
    pub fn with_dirs(dirs: Vec<PathBuf>, current_desktop: Vec<String>) -> Self {
        Self {
            dirs,
            locales: vec!["fr".into(), "en".into()],
            current_desktop,
        }
    }

    fn parse_file(&self, path: &Path, dir: &Path) -> Option<(String, AppEntry)> {
        let entry = DesktopEntry::from_path(path, Some(&self.locales)).ok()?;

        // Filtres de visibilité de la spec freedesktop.
        if entry.type_() != Some("Application") || entry.no_display() || entry.hidden() {
            return None;
        }
        if let Some(not_in) = entry.not_show_in() {
            if not_in.iter().any(|d| self.current_desktop.iter().any(|c| c == d)) {
                return None;
            }
        }
        if let Some(only_in) = entry.only_show_in() {
            if !only_in.iter().any(|d| self.current_desktop.iter().any(|c| c == d)) {
                return None;
            }
        }
        // Les apps terminal nécessitent un émulateur de terminal : hors
        // périmètre de la brique 1.
        if entry.terminal() {
            return None;
        }

        let exec = entry.exec()?;
        let file_id = desktop_file_id(path, dir);
        let name = entry
            .name(&self.locales)
            .map(|n| n.into_owned())
            .unwrap_or_else(|| file_id.trim_end_matches(".desktop").to_owned());

        // Raccourci Steam : on délègue le lancement au client Steam et on
        // prend l'appid comme identité, pour dédupliquer avec le scan des
        // manifests Steam.
        let (id, launch, source) = if let Some(app_id) = steam_rungameid(exec) {
            (format!("steam:{app_id}"), LaunchSpec::SteamApp(app_id), Source::Steam)
        } else {
            let argv = clean_exec(exec)?;
            let source = classify(&entry, exec, path);
            (format!("desktop:{file_id}"), LaunchSpec::Exec(argv), source)
        };

        let icon = entry.icon().map(|i| {
            if i.starts_with('/') {
                Icon::Path(PathBuf::from(i))
            } else {
                Icon::Named(i.to_owned())
            }
        });

        let keywords = entry
            .keywords(&self.locales)
            .map(|ks| ks.into_iter().map(|k| k.into_owned()).collect())
            .unwrap_or_default();

        Some((
            file_id,
            AppEntry {
                id,
                name,
                description: entry.comment(&self.locales).map(|c| c.into_owned()),
                icon,
                launch,
                source,
                keywords,
            },
        ))
    }
}

impl AppSource for DesktopScanner {
    fn name(&self) -> &'static str {
        "desktop"
    }

    fn scan(&self) -> Vec<AppEntry> {
        // Même desktop-file-id dans deux répertoires => le premier
        // répertoire (le plus prioritaire) gagne.
        let mut seen: HashMap<String, AppEntry> = HashMap::new();
        let mut order: Vec<String> = Vec::new();

        for dir in &self.dirs {
            for path in walk_desktop_files(dir) {
                if let Some((file_id, entry)) = self.parse_file(&path, dir) {
                    if !seen.contains_key(&file_id) {
                        order.push(file_id.clone());
                        seen.insert(file_id, entry);
                    }
                }
            }
        }

        order.into_iter().filter_map(|id| seen.remove(&id)).collect()
    }
}

/// Répertoires standards, utilisateur d'abord, puis exports Flatpak,
/// puis système ($XDG_DATA_DIRS).
fn default_application_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var("HOME").unwrap_or_default();

    let data_home = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(&home).join(".local/share"));
    dirs.push(data_home.join("applications"));

    // Exports Flatpak explicites : présents même si XDG_DATA_DIRS ne les
    // liste pas (session mal configurée).
    dirs.push(data_home.join("flatpak/exports/share/applications"));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));

    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_owned());
    for d in data_dirs.split(':').filter(|d| !d.is_empty()) {
        dirs.push(Path::new(d).join("applications"));
    }

    dirs.dedup();
    dirs
}

/// Tous les .desktop sous `dir`, récursivement (Wine range les siens dans
/// des sous-répertoires type `wine/Programs/...`).
fn walk_desktop_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else { continue };
        for e in read.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "desktop") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Identifiant freedesktop du fichier : chemin relatif au répertoire
/// `applications/`, séparateurs remplacés par `-`.
fn desktop_file_id(path: &Path, dir: &Path) -> String {
    path.strip_prefix(dir)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("-")
}

/// Nettoie une ligne Exec= : retire les field codes (%f, %u, ...) puis
/// découpe en argv selon les règles de quoting.
fn clean_exec(exec: &str) -> Option<Vec<String>> {
    let mut cleaned = String::with_capacity(exec.len());
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('%') => cleaned.push('%'),
                // Tout autre field code est supprimé (on lance sans
                // fichier/URL en argument).
                Some(_) | None => {}
            }
        } else {
            cleaned.push(c);
        }
    }
    let argv = shell_words::split(&cleaned).ok()?;
    if argv.is_empty() {
        None
    } else {
        Some(argv)
    }
}

/// Extrait l'appid d'un Exec de raccourci Steam (`steam://rungameid/620`).
fn steam_rungameid(exec: &str) -> Option<u32> {
    let idx = exec.find("steam://rungameid/")?;
    let rest = &exec[idx + "steam://rungameid/".len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn classify(entry: &DesktopEntry, exec: &str, path: &Path) -> Source {
    if entry.flatpak().is_some() || exec.trim_start().starts_with("flatpak ") {
        return Source::Flatpak;
    }
    // Raccourcis générés par Wine : `Exec=env WINEPREFIX=... wine ...`,
    // rangés sous applications/wine/.
    let in_wine_dir = path
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("wine"));
    let wine_exec = exec.contains("WINEPREFIX")
        || exec.split_whitespace().any(|tok| {
            let bin = tok.rsplit('/').next().unwrap_or(tok);
            bin == "wine" || bin == "wine64"
        });
    if in_wine_dir || wine_exec {
        return Source::Wine;
    }
    Source::Native
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_exec_strips_field_codes() {
        assert_eq!(clean_exec("firefox %u").unwrap(), vec!["firefox"]);
        assert_eq!(
            clean_exec("env FOO=bar app --flag %F").unwrap(),
            vec!["env", "FOO=bar", "app", "--flag"]
        );
        assert_eq!(clean_exec("app %%v").unwrap(), vec!["app", "%v"]);
    }

    #[test]
    fn clean_exec_handles_quotes() {
        assert_eq!(
            clean_exec(r#""/opt/My App/run" --x %u"#).unwrap(),
            vec!["/opt/My App/run", "--x"]
        );
    }

    #[test]
    fn steam_rungameid_parses() {
        assert_eq!(steam_rungameid("steam steam://rungameid/620"), Some(620));
        assert_eq!(steam_rungameid("firefox %u"), None);
    }
}
