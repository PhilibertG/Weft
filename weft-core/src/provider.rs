//! L'abstraction centrale de l'étape 2 : un Provider répond à la frappe.
//!
//! À ne pas confondre avec `AppSource` (scan d'avance d'une liste d'apps) :
//! un provider peut calculer ses résultats au moment de la requête (calc,
//! fichiers). L'UI ne connaît que `ResultItem` — elle ne sait jamais si un
//! résultat est une app, un calcul ou un fichier.
//!
//! ## Classement inter-providers
//!
//! Comparer un score fuzzy d'app à un score de calculatrice n'a pas de
//! sens : ce sont des unités différentes. Le tri se fait donc par `Tier`
//! (règle de catégorie) d'abord, par `score` ensuite — le score n'est
//! comparé qu'à tier égal, et chaque provider doit le normaliser sur
//! 0..=1000.

use std::io;
use std::path::PathBuf;

use crate::model::{Icon, LaunchSpec};

/// Priorité de catégorie. L'ordre de déclaration EST l'ordre d'affichage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Réponse directe à la requête (calc). Passe toujours en premier.
    Answer,
    /// Résultats de premier rang (apps bien matchées).
    Primary,
    /// Ne doit jamais masquer un résultat Primary (fichiers, matches faibles).
    Fallback,
}

/// Ce qui se passe quand l'utilisateur active un résultat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Launch(LaunchSpec),
    /// À copier dans le presse-papier (le presse-papier appartient à l'UI).
    CopyText(String),
    /// Ouvrir avec l'application par défaut (xdg-open).
    OpenPath(PathBuf),
}

#[derive(Debug, Clone)]
pub struct ResultItem {
    /// Unique au sein du provider (frecency, dédup d'affichage).
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<Icon>,
    pub action: Action,
    pub tier: Tier,
    /// 0..=1000, comparé uniquement entre items de même tier.
    pub score: u32,
}

/// Un répertoire à surveiller pour rafraîchir un provider.
/// `recursive: false` pour les répertoires dont seuls les fichiers de
/// premier niveau nous intéressent (ex. steamapps/ — surveiller récursivement
/// embarquerait les fichiers des jeux eux-mêmes).
#[derive(Debug, Clone)]
pub struct WatchSpec {
    pub path: PathBuf,
    pub recursive: bool,
}

pub trait Provider {
    fn name(&self) -> &'static str;

    /// Recharge les données sous-jacentes (rescan). No-op si sans objet.
    fn refresh(&mut self) {}

    /// Répertoires dont un changement doit déclencher `refresh()`.
    fn watch_specs(&self) -> Vec<WatchSpec> {
        Vec::new()
    }

    /// Résultats pour la requête. Contrat sur la requête vide : mode
    /// "parcourir" — les providers de catalogue (apps) rendent tout, les
    /// providers de réponse (calc, fichiers) ne rendent rien.
    fn query(&mut self, query: &str) -> Vec<ResultItem>;

    /// Un des résultats de ce provider a été activé (base de la frecency ;
    /// c'est au provider de s'en occuper, pas au cœur).
    fn record_activation(&mut self, item_id: &str) {
        let _ = item_id;
    }
}

/// Un résultat prêt pour l'UI, avec de quoi router l'activation vers le
/// provider qui l'a produit.
#[derive(Debug, Clone)]
pub struct Hit {
    provider_idx: usize,
    pub item: ResultItem,
}

/// Réponse d'une activation : ce que l'UI doit encore faire elle-même.
#[derive(Debug, PartialEq, Eq)]
pub enum Activation {
    Done,
    /// Le cœur n'a pas accès au presse-papier ; l'UI copie ce texte.
    CopyRequested(String),
}

/// L'ensemble des providers actifs, dans un ordre stable.
#[derive(Default)]
pub struct Registry {
    providers: Vec<Box<dyn Provider>>,
}

impl Registry {
    pub fn new(providers: Vec<Box<dyn Provider>>) -> Self {
        Self { providers }
    }

    /// Registre par défaut du système réel (la config le rendra pilotable).
    pub fn with_defaults() -> Self {
        Self::new(vec![Box::new(crate::providers::apps::AppProvider::new())])
    }

    pub fn refresh(&mut self) {
        for p in &mut self.providers {
            p.refresh();
        }
    }

    pub fn watch_specs(&self) -> Vec<WatchSpec> {
        self.providers.iter().flat_map(|p| p.watch_specs()).collect()
    }

    /// Interroge tous les providers et fusionne : tier, puis score
    /// décroissant, puis titre (ordre stable).
    pub fn query(&mut self, query: &str) -> Vec<Hit> {
        let mut hits: Vec<Hit> = Vec::new();
        for (provider_idx, p) in self.providers.iter_mut().enumerate() {
            hits.extend(
                p.query(query)
                    .into_iter()
                    .map(|item| Hit { provider_idx, item }),
            );
        }
        hits.sort_by(|a, b| {
            a.item
                .tier
                .cmp(&b.item.tier)
                .then(b.item.score.cmp(&a.item.score))
                .then_with(|| a.item.title.cmp(&b.item.title))
        });
        hits
    }

    /// Exécute l'action du hit et notifie le provider d'origine.
    pub fn activate(&mut self, hit: &Hit) -> io::Result<Activation> {
        let outcome = match &hit.item.action {
            Action::Launch(spec) => {
                crate::launch::launch_spec(spec)?;
                Activation::Done
            }
            Action::OpenPath(path) => {
                crate::launch::open_path(path)?;
                Activation::Done
            }
            Action::CopyText(text) => Activation::CopyRequested(text.clone()),
        };
        if let Some(p) = self.providers.get_mut(hit.provider_idx) {
            p.record_activation(&hit.item.id);
        }
        Ok(outcome)
    }
}
