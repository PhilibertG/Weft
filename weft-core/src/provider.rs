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

/// Comment défaire l'installation d'une app. Volontairement limité aux
/// sources où la désinstallation est SÛRE : pas de mot de passe root, pas
/// de risque d'emporter des dépendances système. Les apps natives (apt,
/// AppImage posé à la main…) n'ont pas d'entrée ici — elles ne sont pas
/// désinstallables depuis le launcher, et c'est assumé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UninstallSpec {
    /// App Windows installée par Weft : suppression totale du préfixe isolé
    /// (slug dans le store). Le plus net — tout nous appartient.
    WeftWindows(String),
    /// Application Flatpak (identifiant, ex. `org.gimp.GIMP`) :
    /// `flatpak uninstall`, sans root pour les installs utilisateur.
    Flatpak(String),
    /// Jeu Steam : on ouvre le dialogue de désinstallation du client
    /// (`steam://uninstall/<appid>`) — Steam gère, jamais silencieux.
    Steam(u32),
}

#[derive(Debug, Clone)]
pub struct ResultItem {
    /// Unique au sein du provider (frecency, dédup d'affichage).
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<Icon>,
    pub action: Action,
    /// Désinstallation possible (et sûre) de ce résultat, si applicable.
    /// `None` pour tout ce qui n'est pas une app désinstallable proprement
    /// (calc, fichiers, apps natives…).
    pub uninstall: Option<UninstallSpec>,
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

    /// Registre du système réel, providers activés selon la config.
    pub fn from_config(cfg: &crate::config::ProvidersConfig) -> Self {
        let mut providers: Vec<Box<dyn Provider>> = Vec::new();
        if cfg.apps {
            providers.push(Box::new(crate::providers::apps::AppProvider::new()));
        }
        if cfg.calc {
            providers.push(Box::new(crate::providers::calc::CalcProvider));
        }
        if cfg.files {
            providers.push(Box::new(crate::providers::files::FilesProvider::new()));
        }
        Self::new(providers)
    }

    /// Registre par défaut (tous les providers).
    pub fn with_defaults() -> Self {
        Self::from_config(&crate::config::ProvidersConfig::default())
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

    /// Désinstalle le résultat s'il est désinstallable. Erreur explicite si
    /// le résultat n'expose aucune méthode sûre (jamais présenté comme tel
    /// par l'UI, mais on refuse proprement plutôt que d'échouer en silence).
    pub fn uninstall(&mut self, hit: &Hit) -> io::Result<()> {
        match &hit.item.uninstall {
            Some(spec) => crate::launch::uninstall(spec),
            None => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "cette app ne se désinstalle pas depuis Weft",
            )),
        }
    }
}
