use std::path::PathBuf;

/// Where an app comes from. Internal metadata only: the UI must never
/// surface this in the main list — that is the whole point of Weft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Native,
    Flatpak,
    Steam,
    Wine,
}

/// An icon is either a themed name (resolved by the icon theme at render
/// time) or a concrete file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Icon {
    Named(String),
    Path(PathBuf),
}

/// How to start the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchSpec {
    /// Ready-to-spawn argv (Exec line with field codes stripped).
    Exec(Vec<String>),
    /// Launched through the Steam client (covers Proton transparently).
    SteamApp(u32),
    /// App Windows installée par Weft, lancée via umu/Proton (brique 2).
    /// La string est le slug dans le store d'apps Windows.
    WindowsApp(String),
}

/// The unified app object. Every source maps into this and nothing else.
#[derive(Debug, Clone)]
pub struct AppEntry {
    /// Stable unique id, e.g. "desktop:firefox.desktop" or "steam:620".
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<Icon>,
    pub launch: LaunchSpec,
    pub source: Source,
    /// Extra search terms (Keywords= from .desktop files).
    pub keywords: Vec<String>,
}
