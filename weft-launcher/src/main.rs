//! CLI de debug en attendant l'UI GTK :
//!   weft-launcher            -> liste tout
//!   weft-launcher <requête>  -> recherche fuzzy

use weft_core::search::Searcher;
use weft_core::Index;

fn main() {
    let query = std::env::args().nth(1).unwrap_or_default();

    let start = std::time::Instant::now();
    let index = Index::build();
    let scan_time = start.elapsed();

    let mut searcher = Searcher::new();
    let hits = searcher.search(index.entries(), &query);

    for &i in hits.iter().take(15) {
        let e = &index.entries()[i];
        println!("{:<40} [{:?}]", e.name, e.source);
    }
    eprintln!(
        "\n{} apps indexées en {:?}, {} résultats",
        index.len(),
        scan_time,
        hits.len()
    );
}
