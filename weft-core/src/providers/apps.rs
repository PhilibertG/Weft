//! Le provider des applications : l'existant (Index + Searcher) derrière
//! l'interface Provider. La frecency vivra ici (responsabilité du provider,
//! pas du cœur).

use crate::index::Index;
use crate::model::AppEntry;
use crate::provider::{Action, Provider, ResultItem, Tier};
use crate::search::Searcher;

pub struct AppProvider {
    index: Index,
    searcher: Searcher,
    /// En tests : index figé, pas de rescan système sur refresh().
    frozen: bool,
}

impl AppProvider {
    pub fn new() -> Self {
        Self {
            index: Index::build(),
            searcher: Searcher::new(),
            frozen: false,
        }
    }

    /// Provider sur un index fourni (tests/fixtures).
    pub fn from_index(index: Index) -> Self {
        Self {
            index,
            searcher: Searcher::new(),
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
        self.searcher
            .search_scored(self.index.entries(), query)
            .into_iter()
            .map(|(i, score)| Self::to_item(&self.index.entries()[i], score))
            .collect()
    }
}
