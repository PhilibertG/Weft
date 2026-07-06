//! Tests du système de providers : fusion par tiers, routage d'activation.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use weft_core::providers::apps::AppProvider;
use weft_core::sources::desktop::DesktopScanner;
use weft_core::{Action, Activation, Index, Provider, Registry, ResultItem, Tier};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn app_provider() -> AppProvider {
    let scanner = DesktopScanner::with_dirs(
        vec![
            fixtures().join("applications-user"),
            fixtures().join("applications-system"),
        ],
        vec!["GNOME".to_owned()],
    );
    AppProvider::from_index(Index::from_sources(&[&scanner]))
}

/// Provider factice type calculatrice : répond en tier Answer aux requêtes
/// qui commencent par '=', enregistre les activations.
struct FakeAnswerProvider {
    activated: Rc<RefCell<Vec<String>>>,
}

impl Provider for FakeAnswerProvider {
    fn name(&self) -> &'static str {
        "fake-answer"
    }

    fn query(&mut self, query: &str) -> Vec<ResultItem> {
        if !query.starts_with('=') {
            return Vec::new();
        }
        vec![ResultItem {
            id: "answer".into(),
            title: "42".into(),
            subtitle: None,
            icon: None,
            action: Action::CopyText("42".into()),
            tier: Tier::Answer,
            // Score volontairement bas : le tier doit suffire à le mettre
            // devant les apps, c'est exactement ce qu'on teste.
            score: 1,
        }]
    }

    fn record_activation(&mut self, item_id: &str) {
        self.activated.borrow_mut().push(item_id.to_owned());
    }
}

#[test]
fn answer_tier_beats_high_scoring_apps() {
    let activated = Rc::new(RefCell::new(Vec::new()));
    let mut registry = Registry::new(vec![
        Box::new(app_provider()),
        Box::new(FakeAnswerProvider { activated: activated.clone() }),
    ]);

    // "=fire" matche des apps (score élevé) ET le fake provider (score 1).
    let hits = registry.query("=fire");
    assert!(!hits.is_empty());
    assert_eq!(hits[0].item.title, "42", "le tier Answer doit passer devant");
    assert_eq!(hits[0].item.tier, Tier::Answer);
}

#[test]
fn activation_routes_to_owning_provider() {
    let activated = Rc::new(RefCell::new(Vec::new()));
    let mut registry = Registry::new(vec![
        Box::new(app_provider()),
        Box::new(FakeAnswerProvider { activated: activated.clone() }),
    ]);

    let hits = registry.query("=x");
    let outcome = registry.activate(&hits[0]).unwrap();

    // CopyText remonte à l'UI (le cœur n'a pas de presse-papier)…
    assert_eq!(outcome, Activation::CopyRequested("42".into()));
    // …et c'est le provider d'origine qui a été notifié, pas un autre.
    assert_eq!(*activated.borrow(), vec!["answer".to_owned()]);
}

#[test]
fn app_provider_maps_entries_to_items() {
    let mut apps = app_provider();

    let items = apps.query("ffx");
    assert_eq!(items[0].title, "Firefox (user override)");
    assert_eq!(items[0].tier, Tier::Primary);
    assert!(items[0].score > 0);
    assert!(matches!(items[0].action, Action::Launch(_)));

    // Requête vide : mode parcourir, tout est là.
    assert!(apps.query("").len() >= 4);
    // Bruit : rien.
    assert!(apps.query("zzzzqqqq").is_empty());
}
