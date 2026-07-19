<div align="center">

# Weft

**Un seul launcher pour tous vos mondes.**

Apps Linux natives, Flatpak, jeux Steam, programmes Windows — une seule liste,
une seule recherche, zéro jargon. Weft rend la cohabitation des écosystèmes
invisible : c'est la *couture invisible*.

</div>

## Le concept

Sur un bureau Linux, chaque écosystème se trahit : les jeux Windows passent
par un client, les apps Wine hurlent leur origine, chaque monde a son
gestionnaire, sa friction, son jargon. Weft fait disparaître ces coutures
sur le bureau que vous avez déjà : une application se cherche, se lance,
s'installe et se désinstalle de la même façon, qu'elle soit native,
Flatpak, Steam, GOG, Epic ou un simple `.exe`, sans que sa provenance
n'apparaisse jamais.

Le projet est né avec l'ambition d'un OS complet ; l'expérience a tranché
autrement : la couture invisible n'a pas besoin de remplacer votre
environnement, elle fonctionne dedans. Weft est donc un outil qui
s'installe en un paquet sur votre distribution, pas un système qui vous
demande de déménager.

Weft fournit deux composants :

- **Le launcher unifié** — un overlay façon Spotlight, invoqué au clavier,
  qui indexe toutes vos applications quelle que soit leur origine. La source
  d'une app n'apparaît jamais dans l'interface : un jeu Windows lancé via
  Proton se présente exactement comme Firefox.
- **Le monde Windows sans voir Wine** — double-cliquez un `.exe` ou un
  `.msi` dans votre gestionnaire de fichiers : Weft détecte le type
  d'installeur, l'exécute dans un environnement isolé, extrait l'icône, et
  l'application apparaît dans le launcher comme n'importe quelle autre.
  Ni Wine, ni Proton, ni préfixe ne sont jamais mentionnés.

## Fonctionnalités

**Launcher**

- Recherche fuzzy instantanée (moteur nucleo, celui de l'éditeur Helix),
  tolérante aux fautes, matching sur les mots-clés (« navigateur » trouve
  Firefox)
- Classement par fréquence d'usage à décroissance temporelle : vos apps
  du moment remontent, les oubliées redescendent
- Calculatrice inline : tapez `23*7` ou `100 km en miles`, Entrée copie le
  résultat
- Recherche de fichiers (via `plocate`), toujours sous les applications —
  un fichier obscur ne masque jamais une app évidente
- Navigation 100 % clavier : flèches, Entrée, Échap, <kbd>Ctrl</kbd>+<kbd>Suppr</kbd>
- Désinstallation sans quitter le launcher : <kbd>Ctrl</kbd>+<kbd>Suppr</kbd> sur
  la ligne sélectionnée, confirmation au clavier, l'app disparaît de la liste.
  Limitée aux sources où la suppression est sûre et sans mot de passe (voir
  [Utilisation](#utilisation)) — les paquets système ne sont pas concernés
- Process résident : apparition en ~20 ms, index reconstruit automatiquement
  quand vous installez ou supprimez une app (apt, Flatpak, Steam, Windows)

**Sources d'applications**

- `.desktop` natifs, exports Flatpak, raccourcis Wine (avec déduplication
  intelligente : la même app en `.deb` et en Flatpak n'apparaît qu'une fois)
- Bibliothèques Steam, y compris les jeux Windows via Proton (l'outillage
  Proton/runtimes est filtré), clients natif et Flatpak supportés
- Programmes Windows installés par Weft
- Jeux **GOG** et **Epic Games** — sans installer leurs launchers

**Moteur Windows**

- Détection automatique du type : installeurs Inno Setup, NSIS, MSI, ou
  exécutable portable
- Un environnement isolé par application : une app cassée n'en contamine
  jamais une autre
- Runtime [umu](https://github.com/Open-Wine-Components/umu-launcher) +
  UMU-Proton à **versions épinglées** (vérifiées sha512) — jamais de
  « latest » implicite, les correctifs par-jeu de protonfixes inclus
- Aucune dépendance i386 sur l'hôte : tout tourne dans le conteneur Steam
  Linux Runtime
- Icônes extraites des ressources des exécutables
- Jeux GOG : téléchargement de l'installeur offline officiel
  (lgogdownloader) puis installation silencieuse — DRM-free, aucun client
- Jeux Epic : téléchargement et authentification via legendary, correctifs
  par-jeu [protonfixes](https://github.com/Open-Wine-Components/umu-protonfixes)
  résolus automatiquement depuis la base umu
- Réglages par-app automatiques : un jeu 32 bits sur un hôte sans pilotes
  Vulkan i386 bascule tout seul en rendu OpenGL
- Échec d'installation → message honnête et nettoyage complet, jamais de
  stacktrace ni d'app fantôme

## Prérequis

- Linux avec Wayland ou X11 — développé et testé sur Ubuntu 24.04 / GNOME
- Les dépendances runtime (GTK4, libadwaita, python3…) sont tirées
  automatiquement par le paquet `.deb`
- Optionnels, avec dégradation propre s'ils manquent (l'assistant ou le
  paquet les recommandent) :
  - `plocate` — recherche de fichiers
  - `icoutils` — extraction d'icônes des programmes Windows
  - [legendary](https://github.com/legendary-gl/legendary)
    (`pipx install legendary-gl`) — jeux Epic Games
  - [lgogdownloader](https://github.com/Sude-/lgogdownloader) **≥ 3.15** —
    jeux GOG (le 3.12 des dépôts Ubuntu 24.04 a un bug fatal sur les
    métadonnées de jeux ; compilez une version récente)
  - pilotes graphiques 32 bits (`libgl1:i386`, `mesa-vulkan-drivers:i386`)
    pour les jeux Windows 32 bits — sinon bascule OpenGL automatique,
    voire échec si l'hôte n'a aucun pilote i386
- Pour **construire depuis les sources** : [Rust](https://rustup.rs)
  (édition 2021) et `sudo apt install libgtk-4-dev libadwaita-1-dev`

## Installation

Téléchargez le `.deb` depuis la
[dernière release](https://github.com/PhilibertG/Weft/releases/latest),
puis :

```bash
sudo apt install ./weft_*_amd64.deb
```

Le paquet embarque les trois binaires (aucune toolchain Rust requise), le
service de session, le profil AppArmor et le handler `.exe`/`.msi`.

Lancez ensuite **Weft** depuis la grille d'applications : un **assistant de
premier lancement** configure ce qui relève de votre session — raccourci
clavier (<kbd>Super</kbd>+<kbd>Entrée</kbd>), démarrage automatique,
association des programmes Windows — et propose de télécharger
l'environnement Windows. Tout y est facultatif et idempotent ; on peut le
relancer avec `weft-launcher --setup`.

### Construire depuis les sources

```bash
git clone https://github.com/PhilibertG/Weft.git
cd Weft
cargo install cargo-deb
cargo build --release
cargo deb -p weft-launcher --no-build   # → target/debian/weft_*.deb
```

## Utilisation

Invoquez le launcher avec votre raccourci, tapez, Entrée. C'est tout.

| Vous tapez | Vous obtenez |
|---|---|
| `fire` | Firefox, en tête si vous l'utilisez souvent |
| `23*7` | `= 161`, Entrée pour copier |
| `2 inches en cm` | `= 5.08 cm` |
| `rapport` | vos fichiers, sous les applications |

### Désinstaller une application

Sélectionnez-la et pressez <kbd>Ctrl</kbd>+<kbd>Suppr</kbd> : une barre de
confirmation apparaît, <kbd>Entrée</kbd> valide, <kbd>Échap</kbd> annule.
L'indice `⌦ désinstaller` sur la ligne sélectionnée signale les applications
éligibles.

| Origine de l'app | Ce que fait Weft |
|---|---|
| Programme Windows installé par Weft | supprime l'application et son environnement isolé |
| Application Flatpak | `flatpak uninstall` |
| Jeu Steam | ouvre le dialogue de désinstallation du client Steam |
| Paquet système (apt), AppImage… | **non pris en charge** |

> [!NOTE]
> Seules les origines désinstallables proprement et **sans droits root** sont
> proposées. Une application installée par apt — Blender, VS Code… — n'est pas
> désinstallable depuis Weft : passez par votre gestionnaire de paquets. Ce
> n'est pas une limite technique mais un choix : retirer un paquet système
> peut en emporter d'autres, ça ne se fait pas derrière une confirmation d'une
> seconde.

Le monde Windows se pilote aussi en CLI :

```bash
weft-windows runtime status      # état du runtime (umu, Proton, conteneur)
weft-windows runtime fetch       # téléchargement explicite (~1 Go, une fois)
weft-windows install app.exe     # installer (installeur ou portable)
weft-windows list                # apps installées
weft-windows run <app>           # lancer
weft-windows remove <app>        # supprimer (environnement compris)

weft-windows epic login          # connexion Epic (une fois, via navigateur)
weft-windows epic list           # bibliothèque Epic
weft-windows epic install <jeu>  # installer un jeu Epic
weft-windows gog login           # connexion GOG (une fois)
weft-windows gog list            # bibliothèque GOG
weft-windows gog install <jeu>   # installer un jeu GOG
```

Les jeux installés apparaissent dans le launcher comme tout le reste.

> [!NOTE]
> Le premier usage du monde Windows télécharge le runtime (~1 Go). L'UI
> d'installation le propose automatiquement ; les logs de chaque app vont
> dans son dossier, jamais à l'écran.

## Configuration

`~/.config/weft/config.toml`, créé commenté au premier lancement :

```toml
[window]
width = 620
height = 440
max_results = 8

[providers]
apps = true   # applications
calc = true   # calculatrice inline
files = true  # recherche de fichiers (nécessite plocate)
```

## Architecture

Workspace Rust, quatre crates :

```
weft-core/        bibliothèque pure, sans dépendance UI
  sources/        scanners d'apps (.desktop, Steam, Windows Weft)
  providers/      répondent à la frappe (apps, calc, fichiers)
  windows/        moteur Windows (runtime, préfixes, install, icônes)
weft-launcher/    overlay GTK4/libadwaita + daemon
weft-installer/   UI d'installation des programmes Windows
weft-windows/     CLI du moteur Windows
```

Deux abstractions structurent le cœur : les **sources** (scan d'avance des
apps installées) et les **providers** (résultats calculés à la frappe).
L'UI ne connaît que des `ResultItem` génériques — elle ignore tout de leur
origine. Le classement inter-providers est par *tiers* (réponse directe >
applications > fichiers), le score fuzzy n'étant comparé qu'à tier égal :
c'est ce qui évite le launcher agaçant où un fichier passe devant l'app
évidente.

Les tests unitaires et d'intégration tournent sur fixtures (`cargo test`),
chaque étape étant ensuite validée sur machine réelle.

## Feuille de route

Fait :

- [x] **Launcher unifié** : sources natives/Flatpak/Steam/Wine, fuzzy +
  frecency, calc, fichiers, daemon, config, désinstallation, style
- [x] **Monde Windows** : moteur umu/Proton épinglé, préfixes isolés,
  install par double-clic, icônes
- [x] **GOG et Epic sans leurs launchers** : bibliothèques, installation et
  lancement via legendary/lgogdownloader, correctifs protonfixes par-jeu
- [x] **Distribution** : paquet `.deb` en release, assistant de premier
  lancement

En cours / à venir :

- [ ] **Échecs connus** : détecter avant installation les programmes
  notoirement incompatibles (Office Click-to-Run, anticheat kernel…) et
  afficher un message honnête avec des alternatives, plutôt que de laisser
  l'installeur échouer de façon cryptique

Pistes explorées puis écartées — le projet visait initialement un OS
complet (environnement de bureau dédié, image système bootable) ; la
conclusion de l'expérience est que la couture invisible fonctionne mieux
en outil qui s'intègre au bureau existant qu'en système qui le remplace.
Ces chantiers ne sont pas prévus mais si cela vient à vous manquez je tenterais
l'aventure !

> [!WARNING]
> Projet personnel en développement actif, testé principalement sur
> Ubuntu 24.04 / GNOME / Wayland. Les API internes bougent encore.