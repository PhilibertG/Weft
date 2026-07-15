//! Provider calculatrice : taper "23*7" affiche 161, style Spotlight.
//!
//! Moteur : fend (pure Rust, précision arbitraire, unités — "2 pouces en
//! cm" marche aussi). On ne tente l'évaluation que si la requête contient
//! un chiffre : inutile de payer un parse de "firefox", et une requête sans
//! chiffre qui s'évaluerait par accident (nom de constante...) serait plus
//! surprenante qu'utile.

use crate::model::Icon;
use crate::provider::{Action, Provider, ResultItem, Tier};

pub struct CalcProvider;

impl Provider for CalcProvider {
    fn name(&self) -> &'static str {
        "calc"
    }

    fn query(&mut self, query: &str) -> Vec<ResultItem> {
        let query = query.trim();
        if !query.chars().any(|c| c.is_ascii_digit()) {
            return Vec::new();
        }

        // fend ne comprend que l'anglais : « 100 km en miles » → « to ».
        // Suffisant pour les unités aux noms identiques (km, kg, cm, l...) ;
        // pas de table de traduction d'unités, hors de proportion ici.
        let translated: String = query
            .split_whitespace()
            .map(|w| if w == "en" { "to" } else { w })
            .collect::<Vec<_>>()
            .join(" ");

        let mut ctx = fend_core::Context::new();
        let Ok(result) = fend_core::evaluate(&translated, &mut ctx) else {
            return Vec::new();
        };
        let value = result.get_main_result().to_owned();

        // "42" => "42" : aucune information, on se tait.
        if value.is_empty() || value == query {
            return Vec::new();
        }

        vec![ResultItem {
            id: format!("calc:{query}"),
            title: format!("= {value}"),
            subtitle: Some("Entrée pour copier".to_owned()),
            icon: Some(Icon::Named("accessories-calculator".to_owned())),
            action: Action::CopyText(value),
            uninstall: None,
            tier: Tier::Answer,
            score: 1000,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(query: &str) -> Option<ResultItem> {
        CalcProvider.query(query).into_iter().next()
    }

    #[test]
    fn evaluates_arithmetic() {
        let item = eval("23*7").unwrap();
        assert_eq!(item.title, "= 161");
        assert_eq!(item.tier, Tier::Answer);
        assert_eq!(item.action, Action::CopyText("161".into()));
    }

    #[test]
    fn converts_units() {
        let item = eval("2 inches to cm").unwrap();
        assert!(item.title.contains("5.08"), "obtenu : {}", item.title);
        // « en » français traduit vers « to ».
        let item = eval("100 km en miles").unwrap();
        assert!(item.title.contains("62.1"), "obtenu : {}", item.title);
    }

    #[test]
    fn stays_quiet_when_not_math() {
        assert!(eval("firefox").is_none()); // pas de chiffre
        assert!(eval("vlc 2").is_none()); // pas évaluable
        assert!(eval("42").is_none()); // résultat = requête
        assert!(eval("").is_none());
    }
}
