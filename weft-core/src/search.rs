//! Recherche fuzzy sur l'index.
//!
//! On utilise le matcher de nucleo (celui de l'éditeur Helix) : tolérant aux
//! fautes d'ordre ("code vis" trouve "Visual Studio Code"), scoring qui
//! favorise les débuts de mots, et largement assez rapide pour re-scorer
//! quelques centaines d'apps à chaque frappe.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::model::AppEntry;

pub struct Searcher {
    matcher: Matcher,
}

impl Default for Searcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Searcher {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    /// Indices dans `entries`, du meilleur score au moins bon.
    pub fn search(&mut self, entries: &[AppEntry], query: &str) -> Vec<usize> {
        self.search_scored(entries, query)
            .into_iter()
            .map(|(i, _)| i)
            .collect()
    }

    /// Comme `search`, avec le score de chaque hit.
    /// Requête vide => tout, ordre alphabétique, score 0.
    pub fn search_scored(&mut self, entries: &[AppEntry], query: &str) -> Vec<(usize, u32)> {
        if query.trim().is_empty() {
            let mut all: Vec<usize> = (0..entries.len()).collect();
            all.sort_by_key(|&i| entries[i].name.to_lowercase());
            return all.into_iter().map(|i| (i, 0)).collect();
        }

        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored: Vec<(u32, usize)> = entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                self.score(&pattern, entry, &mut buf).map(|s| (s, i))
            })
            .collect();
        // Score décroissant, ties par nom pour un ordre stable.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| entries[a.1].name.cmp(&entries[b.1].name)));
        scored.into_iter().map(|(s, i)| (i, s)).collect()
    }

    /// Score d'une entrée : le nom compte plein pot, les mots-clés comptent
    /// atténués (trouver "navigateur" doit lister Firefox, mais après une
    /// app qui s'appelle vraiment "Navigateur").
    fn score(&mut self, pattern: &Pattern, entry: &AppEntry, buf: &mut Vec<char>) -> Option<u32> {
        let name_score = pattern.score(Utf32Str::new(&entry.name, buf), &mut self.matcher);
        let keyword_score = entry
            .keywords
            .iter()
            .filter_map(|k| pattern.score(Utf32Str::new(k, buf), &mut self.matcher))
            .max()
            .map(|s| s / 2);
        name_score.max(keyword_score)
    }
}
