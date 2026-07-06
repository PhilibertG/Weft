pub mod desktop;
pub mod steam;

use crate::model::AppEntry;

/// A source of applications. Scanners are pure: they read a filesystem
/// layout passed in explicitly (never hardcoded paths), so tests can point
/// them at fixtures.
pub trait AppSource {
    /// Human-readable name, for logs/debug only.
    fn name(&self) -> &'static str;

    /// Scan and return everything found. Errors on individual entries are
    /// swallowed (a broken .desktop file must not kill the index); a scan
    /// only fails as a whole if the source is entirely unreadable.
    fn scan(&self) -> Vec<AppEntry>;
}
