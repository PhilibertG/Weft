//! Frecency : fréquence d'usage avec décroissance temporelle.
//!
//! Chaque activation incrémente un compteur qui se dégrade avec le temps
//! (demi-vie de 7 jours) : une app lancée 50 fois le mois dernier finit par
//! repasser derrière celle lancée 5 fois cette semaine. Persisté dans un
//! fichier texte simple (une ligne par app : id, valeur, dernier usage) —
//! pas de dépendance sérialisation pour trois champs.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Demi-vie de la décroissance, en secondes (7 jours).
const HALF_LIFE_SECS: f64 = 7.0 * 24.0 * 3600.0;

/// Bonus maximal ajouté au score fuzzy (échelle commune 0..=1000 ; un bon
/// match nucleo vaut ~100-300, le bonus peut départager sans écraser).
const MAX_BOOST: f64 = 150.0;

#[derive(Clone, Copy)]
struct Usage {
    value: f64,
    last_secs: u64,
}

pub struct UsageStore {
    /// None => volatile (tests, ou HOME introuvable) : jamais écrit sur disque.
    path: Option<PathBuf>,
    entries: HashMap<String, Usage>,
}

impl UsageStore {
    /// Store persistant dans ~/.local/share/weft/usage.tsv.
    pub fn open_default() -> Self {
        let path = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|d| d.join("weft/usage.tsv"))
            .ok();
        Self::open(path)
    }

    pub fn open(path: Option<PathBuf>) -> Self {
        let mut entries = HashMap::new();
        if let Some(p) = &path {
            if let Ok(text) = std::fs::read_to_string(p) {
                for line in text.lines() {
                    let mut parts = line.split('\t');
                    if let (Some(id), Some(v), Some(t)) =
                        (parts.next(), parts.next(), parts.next())
                    {
                        if let (Ok(value), Ok(last_secs)) = (v.parse(), t.parse()) {
                            entries.insert(id.to_owned(), Usage { value, last_secs });
                        }
                    }
                }
            }
        }
        Self { path, entries }
    }

    pub fn in_memory() -> Self {
        Self::open(None)
    }

    pub fn record(&mut self, id: &str) {
        self.record_at(id, now_secs());
        self.save();
    }

    /// Bonus de score (0..=MAX_BOOST) à ajouter au score fuzzy.
    pub fn boost(&self, id: &str) -> u32 {
        self.boost_at(id, now_secs())
    }

    fn record_at(&mut self, id: &str, now: u64) {
        let u = self.entries.entry(id.to_owned()).or_insert(Usage {
            value: 0.0,
            last_secs: now,
        });
        u.value = decayed(u.value, u.last_secs, now) + 1.0;
        u.last_secs = now;
    }

    fn boost_at(&self, id: &str, now: u64) -> u32 {
        let Some(u) = self.entries.get(id) else { return 0 };
        let v = decayed(u.value, u.last_secs, now);
        // Saturation douce : 1 usage ≈ 25, 5 ≈ 75, beaucoup → plafonne.
        (MAX_BOOST * v / (v + 5.0)) as u32
    }

    /// Écriture atomique (fichier temporaire + rename) : un crash pendant
    /// la sauvegarde ne corrompt jamais l'historique.
    fn save(&self) {
        let Some(path) = &self.path else { return };
        let Some(dir) = path.parent() else { return };
        let _ = std::fs::create_dir_all(dir);
        let tmp = path.with_extension("tsv.tmp");
        let mut out = String::new();
        for (id, u) in &self.entries {
            out.push_str(&format!("{id}\t{}\t{}\n", u.value, u.last_secs));
        }
        let ok = std::fs::File::create(&tmp)
            .and_then(|mut f| f.write_all(out.as_bytes()))
            .is_ok();
        if ok {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

fn decayed(value: f64, last_secs: u64, now: u64) -> f64 {
    let dt = now.saturating_sub(last_secs) as f64;
    value * 0.5_f64.powf(dt / HALF_LIFE_SECS)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 3600;

    #[test]
    fn boost_grows_with_use_and_saturates() {
        let mut s = UsageStore::in_memory();
        assert_eq!(s.boost_at("app", 0), 0);
        s.record_at("app", 0);
        let one = s.boost_at("app", 0);
        for _ in 0..100 {
            s.record_at("app", 0);
        }
        let many = s.boost_at("app", 0);
        assert!(one > 0 && many > one);
        assert!(many <= MAX_BOOST as u32);
    }

    #[test]
    fn recent_use_beats_heavy_old_use() {
        let mut s = UsageStore::in_memory();
        for _ in 0..50 {
            s.record_at("ancienne", 0);
        }
        let now = 30 * DAY;
        for _ in 0..5 {
            s.record_at("recente", now);
        }
        assert!(s.boost_at("recente", now) > s.boost_at("ancienne", now));
    }

    #[test]
    fn persists_across_reopen() {
        let path = std::env::temp_dir().join(format!("weft-usage-{}.tsv", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut s = UsageStore::open(Some(path.clone()));
        s.record("app");
        drop(s);

        let s2 = UsageStore::open(Some(path.clone()));
        assert!(s2.boost("app") > 0);
        let _ = std::fs::remove_file(&path);
    }
}
