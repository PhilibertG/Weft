//! Tests d'intégration : scan des fixtures, dédup, recherche.

use std::path::PathBuf;

use weft_core::model::{LaunchSpec, Source};
use weft_core::search::Searcher;
use weft_core::sources::desktop::DesktopScanner;
use weft_core::sources::steam::SteamScanner;
use weft_core::sources::AppSource;
use weft_core::Index;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn desktop_scanner() -> DesktopScanner {
    DesktopScanner::with_dirs(
        vec![
            fixtures().join("applications-user"),
            fixtures().join("flatpak-exports"),
            fixtures().join("applications-system"),
        ],
        vec!["GNOME".to_owned()],
    )
}

/// Construit une arborescence Steam dans un répertoire temporaire, car
/// libraryfolders.vdf exige des chemins absolus.
fn make_steam_root(tag: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("weft-test-steam-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("steamapps")).unwrap();
    std::fs::create_dir_all(root.join("library2/steamapps")).unwrap();

    let write = |rel: &str, content: String| {
        std::fs::write(root.join(rel), content).unwrap();
    };

    write(
        "steamapps/libraryfolders.vdf",
        format!(
            r#""libraryfolders"
{{
	"0"
	{{
		"path"		"{root}"
	}}
	"1"
	{{
		"path"		"{root}/library2"
	}}
}}
"#,
            root = root.display()
        ),
    );

    let manifest = |appid: u32, name: &str, state: u32| {
        format!(
            r#""AppState"
{{
	"appid"		"{appid}"
	"name"		"{name}"
	"StateFlags"		"{state}"
	"installdir"		"{name}"
}}
"#
        )
    };

    // Bibliothèque principale : un jeu installé + Proton (outillage).
    write("steamapps/appmanifest_620.acf", manifest(620, "Portal 2", 4));
    write(
        "steamapps/appmanifest_2805730.acf",
        manifest(2805730, "Proton 9.0 (Beta)", 4),
    );
    // Deuxième bibliothèque : un jeu installé + un en cours de téléchargement.
    write(
        "library2/steamapps/appmanifest_2357570.acf",
        manifest(2357570, "Overwatch 2", 4),
    );
    write(
        "library2/steamapps/appmanifest_570.acf",
        manifest(570, "Dota 2", 1026),
    );

    root
}

#[test]
fn desktop_scan_finds_visible_apps_only() {
    let entries = desktop_scanner().scan();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

    // Visibles.
    assert!(names.contains(&"Firefox (user override)"));
    assert!(names.contains(&"Notepad++"));
    assert!(names.contains(&"Calculatrice")); // Name[fr] prioritaire
    assert!(names.contains(&"Éditeur de texte"));

    // Filtrés : NoDisplay, OnlyShowIn=KDE, Terminal=true.
    assert!(!names.contains(&"Hidden Tool"));
    assert!(!names.contains(&"KDE Settings Thing"));
    assert!(!names.contains(&"Htop"));
}

#[test]
fn desktop_scan_user_overrides_system() {
    let entries = desktop_scanner().scan();
    let firefox: Vec<_> = entries
        .iter()
        .filter(|e| e.id == "desktop:firefox.desktop")
        .collect();
    assert_eq!(firefox.len(), 1);
    assert_eq!(firefox[0].name, "Firefox (user override)");
}

#[test]
fn desktop_scan_classifies_sources() {
    let entries = desktop_scanner().scan();
    let by_name = |n: &str| entries.iter().find(|e| e.name == n).unwrap();

    assert_eq!(by_name("Firefox (user override)").source, Source::Native);
    assert_eq!(by_name("Notepad++").source, Source::Wine);
    assert_eq!(by_name("Calculatrice").source, Source::Flatpak);
    // Raccourci steam://rungameid => Steam, lancé via le client.
    let portal = by_name("Portal 2 (raccourci bureau)");
    assert_eq!(portal.source, Source::Steam);
    assert_eq!(portal.id, "steam:620");
    assert_eq!(portal.launch, LaunchSpec::SteamApp(620));
}

#[test]
fn steam_scan_reads_all_libraries_and_filters() {
    let root = make_steam_root("scan");
    let entries = SteamScanner::with_root(root.clone()).scan();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

    assert!(names.contains(&"Portal 2"));
    assert!(names.contains(&"Overwatch 2")); // deuxième bibliothèque
    assert!(!names.contains(&"Proton 9.0 (Beta)")); // outillage
    assert!(!names.contains(&"Dota 2")); // pas complètement installé
    assert_eq!(entries.len(), 2);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn index_dedups_steam_shortcut_against_manifest() {
    let root = make_steam_root("dedup");
    let steam = SteamScanner::with_root(root.clone());
    let desktop = desktop_scanner();
    let index = Index::from_sources(&[&steam, &desktop]);

    // Portal 2 vu deux fois (manifest + raccourci .desktop) => une entrée,
    // celle du manifest (source prioritaire).
    let portals: Vec<_> = index
        .entries()
        .iter()
        .filter(|e| e.id == "steam:620")
        .collect();
    assert_eq!(portals.len(), 1);
    assert_eq!(portals[0].name, "Portal 2");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn robustness_bad_files_degrade_quietly() {
    let entries = desktop_scanner().scan();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

    // Fichier illisible : ignoré sans crash, les autres passent.
    assert!(!names.iter().any(|n| n.contains("pas un fichier")));
    // Icône à chemin absolu mort : entrée gardée, icône retirée.
    let dead = entries.iter().find(|e| e.name == "Dead Icon App").unwrap();
    assert!(dead.icon.is_none());
}

#[test]
fn steam_scan_ignores_garbage_manifest() {
    let root = make_steam_root("garbage");
    std::fs::write(
        root.join("steamapps/appmanifest_999.acf"),
        "{{{ du bruit \"non parsable",
    )
    .unwrap();

    let entries = SteamScanner::with_root(root.clone()).scan();
    assert_eq!(entries.len(), 2); // Portal 2 + Overwatch 2, rien de plus

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn index_dedups_deb_and_flatpak_of_same_app() {
    let index = Index::from_sources(&[&desktop_scanner()]);
    let gimps: Vec<_> = index
        .entries()
        .iter()
        .filter(|e| e.name == "GIMP")
        .collect();

    assert_eq!(gimps.len(), 1, "une seule entrée GIMP attendue");
    assert_eq!(gimps[0].source, Source::Native, "priorité au natif");
    // Le Flatpak sans équivalent natif reste, lui.
    assert!(index.entries().iter().any(|e| e.name == "Calculatrice"));
}

#[test]
fn search_is_fuzzy_and_ranked() {
    let entries = desktop_scanner().scan();
    let mut searcher = Searcher::new();

    // Fuzzy : lettres non contiguës.
    let hits = searcher.search(&entries, "ffx");
    assert_eq!(entries[hits[0]].name, "Firefox (user override)");

    // Les mots-clés comptent : "navigateur" est un Keyword de Firefox.
    let hits = searcher.search(&entries, "navigateur");
    assert!(hits.iter().any(|&i| entries[i].name == "Firefox (user override)"));

    // Requête vide : tout, trié par nom.
    let all = searcher.search(&entries, "");
    assert_eq!(all.len(), entries.len());

    // Aucun résultat pour du bruit.
    assert!(searcher.search(&entries, "zzzzqqqq").is_empty());
}
