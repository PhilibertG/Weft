//! Le provider des applications : l'existant (Index + Searcher) derrière
//! l'interface Provider. La frecency vivra ici (responsabilité du provider,
//! pas du cœur).

use crate::index::Index;
use crate::model::{AppEntry, LaunchSpec, Source};
use crate::provider::{Action, Provider, ResultItem, Tier, UninstallSpec, WatchSpec};
use crate::providers::usage::UsageStore;
use crate::search::Searcher;
use crate::sources::{desktop, steam};

pub struct AppProvider {
    index: Index,
    searcher: Searcher,
    usage: UsageStore,
    /// En tests : index figé, pas de rescan système sur refresh().
    frozen: bool,
}

impl AppProvider {
    pub fn new() -> Self {
        Self {
            index: Index::build(),
            searcher: Searcher::new(),
            usage: UsageStore::open_default(),
            frozen: false,
        }
    }

    /// Provider sur un index fourni, usage volatile (tests/fixtures).
    pub fn from_index(index: Index) -> Self {
        Self {
            index,
            searcher: Searcher::new(),
            usage: UsageStore::in_memory(),
            frozen: true,
        }
    }

    fn to_item(entry: &AppEntry, score: u32) -> ResultItem {
        ResultItem {
            id: entry.id.clone(),
            title: entry.name.clone(),
            subtitle: entry.description.clone(),
            icon: entry.icon.clone(),
            action: Action::Launch(entry.launch.clone()),
            uninstall: uninstall_spec(entry),
            tier: Tier::Primary,
            // Le score nucleo n'est pas borné : on écrête sur l'échelle
            // commune. Suffisant tant que le tri fin reste intra-provider.
            score: score.min(1000),
        }
    }
}

impl Default for AppProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for AppProvider {
    fn name(&self) -> &'static str {
        "apps"
    }

    fn refresh(&mut self) {
        if !self.frozen {
            self.index = Index::build();
        }
    }

    fn query(&mut self, query: &str) -> Vec<ResultItem> {
        // Score final = fuzzy + bonus de frecency, puis re-tri : une app
        // très utilisée peut doubler un match légèrement meilleur, et en
        // mode parcourir (requête vide) les habituées remontent en tête.
        let mut items: Vec<ResultItem> = self
            .searcher
            .search_scored(self.index.entries(), query)
            .into_iter()
            .map(|(i, score)| {
                let entry = &self.index.entries()[i];
                Self::to_item(entry, score + self.usage.boost(&entry.id))
            })
            .collect();
        items.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.title.cmp(&b.title)));
        items
    }

    fn record_activation(&mut self, item_id: &str) {
        self.usage.record(item_id);
    }

    fn watch_specs(&self) -> Vec<WatchSpec> {
        if self.frozen {
            return Vec::new();
        }
        let mut specs: Vec<WatchSpec> = desktop::default_application_dirs()
            .into_iter()
            // Récursif : Wine range ses .desktop dans des sous-répertoires.
            .map(|path| WatchSpec { path, recursive: true })
            .collect();
        // Non récursif : les manifests sont à la racine de steamapps/, et
        // en dessous il y a les jeux entiers (steamapps/common).
        specs.extend(
            steam::steamapps_dirs()
                .into_iter()
                .map(|path| WatchSpec { path, recursive: false }),
        );
        // Apps Windows Weft : non récursif aussi — en dessous il y a les
        // préfixes Wine, très bavards. La création/suppression d'un
        // répertoire d'app suffit à déclencher le re-scan.
        if let Some(root) = crate::windows::WindowsRoot::open_default() {
            specs.push(WatchSpec { path: root.apps_dir(), recursive: false });
        }
        specs
    }
}

/// Méthode de désinstallation SÛRE pour cette app, ou `None` si aucune
/// (apps natives apt, AppImage posé à la main… — non désinstallables ici).
fn uninstall_spec(entry: &AppEntry) -> Option<UninstallSpec> {
    // App Windows Weft : reconnue à son id (un raccourci Wine arbitraire est
    // aussi Source::Wine mais n'est pas des nôtres — on n'y touche pas).
    if let Some(slug) = entry.id.strip_prefix("weft-windows:") {
        return Some(UninstallSpec::WeftWindows(slug.to_owned()));
    }
    // Steam : le client gère (dialogue), qu'on vienne d'un manifest ou d'un
    // raccourci .desktop — l'appid suffit.
    if let LaunchSpec::SteamApp(app_id) = entry.launch {
        return Some(UninstallSpec::Steam(app_id));
    }
    // Flatpak : l'identifiant vit dans l'Exec `flatpak run [options] <id>`.
    if entry.source == Source::Flatpak {
        if let LaunchSpec::Exec(argv) = &entry.launch {
            return flatpak_app_id(argv).map(UninstallSpec::Flatpak);
        }
    }
    None
}

/// Extrait l'identifiant d'app d'un argv `flatpak run [--opts] <app-id> …` :
/// premier token après `run` qui n'est pas une option.
fn flatpak_app_id(argv: &[String]) -> Option<String> {
    let mut it = argv.iter();
    it.by_ref().find(|a| a.as_str() == "run")?;
    it.find(|a| !a.starts_with('-')).cloned()
}
