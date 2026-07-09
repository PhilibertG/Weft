//! Détection du type d'un programme Windows et exécution de l'installeur.
//!
//! Détection par signatures binaires, pas par extension seule : un .exe
//! peut être un installeur InnoSetup/NSIS ou un programme portable. La
//! distinction pilote le flux : installeur => on l'exécute dans le préfixe
//! puis on découvre ce qu'il a posé ; portable => on copie l'exe tel quel.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerKind {
    /// Installeur Inno Setup (WinMerge, Git for Windows...).
    Inno,
    /// Installeur NSIS (Notepad++...).
    Nsis,
    /// Windows Installer (.msi, conteneur OLE).
    Msi,
    /// Exécutable PE sans marqueur d'installeur : programme portable.
    PortableExe,
    /// Ni PE ni MSI : on ne sait pas quoi en faire.
    Unknown,
}

/// Magic OLE compound document (les .msi sont des conteneurs OLE).
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

pub fn detect(path: &Path) -> io::Result<InstallerKind> {
    let mut file = std::fs::File::open(path)?;

    let mut head = [0u8; 8];
    let n = file.read(&mut head)?;
    if n >= 8 && head == OLE_MAGIC {
        return Ok(InstallerKind::Msi);
    }
    if n < 2 || &head[..2] != b"MZ" {
        return Ok(InstallerKind::Unknown);
    }

    // PE : chercher les marqueurs d'installeurs dans le binaire. Lecture
    // par blocs avec recouvrement (un marqueur peut chevaucher deux blocs).
    file.seek(SeekFrom::Start(0))?;
    if scan_for(&mut file, &[b"Inno Setup", b"JR.Inno.Setup"])? {
        return Ok(InstallerKind::Inno);
    }
    file.seek(SeekFrom::Start(0))?;
    if scan_for(&mut file, &[b"Nullsoft.NSIS", b"NullsoftInst"])? {
        return Ok(InstallerKind::Nsis);
    }
    Ok(InstallerKind::PortableExe)
}

/// Un exécutable PE est-il 32 bits ? (champ Machine du header COFF)
/// None si le fichier n'est pas un PE lisible.
pub fn is_32bit_pe(path: &Path) -> Option<bool> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 0x40];
    file.read_exact(&mut head).ok()?;
    if &head[..2] != b"MZ" {
        return None;
    }
    let e_lfanew = u32::from_le_bytes(head[0x3C..0x40].try_into().ok()?) as u64;
    file.seek(SeekFrom::Start(e_lfanew)).ok()?;
    let mut pe = [0u8; 6]; // "PE\0\0" + Machine (u16)
    file.read_exact(&mut pe).ok()?;
    if &pe[..4] != b"PE\0\0" {
        return None;
    }
    let machine = u16::from_le_bytes([pe[4], pe[5]]);
    match machine {
        0x014C => Some(true),  // i386
        0x8664 | 0xAA64 => Some(false),
        _ => None,
    }
}

fn scan_for(file: &mut std::fs::File, needles: &[&[u8]]) -> io::Result<bool> {
    const CHUNK: usize = 1 << 20; // 1 Mo
    let overlap = needles.iter().map(|n| n.len()).max().unwrap_or(0);
    let mut buf = vec![0u8; CHUNK + overlap];
    let mut carry = 0usize;

    loop {
        let n = file.read(&mut buf[carry..])?;
        if n == 0 {
            return Ok(false);
        }
        let window = &buf[..carry + n];
        if needles.iter().any(|needle| contains(window, needle)) {
            return Ok(true);
        }
        // Garder la fin du bloc pour les marqueurs à cheval.
        let keep = window.len().min(overlap);
        let start = window.len() - keep;
        buf.copy_within(start..start + keep, 0);
        carry = keep;
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str, content: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("weft-inst-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    fn pe_with(marker: &[u8]) -> Vec<u8> {
        let mut bytes = b"MZ".to_vec();
        bytes.extend_from_slice(&[0u8; 4096]); // padding avant le marqueur
        bytes.extend_from_slice(marker);
        bytes.extend_from_slice(&[0u8; 512]);
        bytes
    }

    #[test]
    fn detects_each_kind() {
        assert_eq!(
            detect(&fixture("a.msi", &{
                let mut b = OLE_MAGIC.to_vec();
                b.extend_from_slice(&[0u8; 64]);
                b
            }))
            .unwrap(),
            InstallerKind::Msi
        );
        assert_eq!(
            detect(&fixture("inno.exe", &pe_with(b"Inno Setup Setup Data"))).unwrap(),
            InstallerKind::Inno
        );
        assert_eq!(
            detect(&fixture("nsis.exe", &pe_with(b"Nullsoft.NSIS.exehead"))).unwrap(),
            InstallerKind::Nsis
        );
        assert_eq!(
            detect(&fixture("plain.exe", &pe_with(b"rien de special"))).unwrap(),
            InstallerKind::PortableExe
        );
        assert_eq!(
            detect(&fixture("texte.txt", b"bonjour")).unwrap(),
            InstallerKind::Unknown
        );
    }

    #[test]
    fn pe_bitness_is_detected() {
        // PE synthétique minimal : MZ, e_lfanew=0x40, "PE\0\0", Machine.
        let make_pe = |machine: u16| {
            let mut b = vec![0u8; 0x46];
            b[0] = b'M';
            b[1] = b'Z';
            b[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
            b[0x40..0x44].copy_from_slice(b"PE\0\0");
            b[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
            b
        };
        assert_eq!(is_32bit_pe(&fixture("pe32.exe", &make_pe(0x014C))), Some(true));
        assert_eq!(is_32bit_pe(&fixture("pe64.exe", &make_pe(0x8664))), Some(false));
        assert_eq!(is_32bit_pe(&fixture("pas-pe.txt", b"bonjour")), None);
    }

    #[test]
    fn marker_straddling_chunks_is_found() {
        // Marqueur placé pile à la frontière d'un bloc de 1 Mo.
        let mut bytes = b"MZ".to_vec();
        bytes.extend_from_slice(&vec![0u8; (1 << 20) - 7]);
        bytes.extend_from_slice(b"Inno Setup");
        assert_eq!(
            detect(&fixture("frontiere.exe", &bytes)).unwrap(),
            InstallerKind::Inno
        );
    }
}
