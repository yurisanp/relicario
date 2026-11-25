//! Emuladores com pastas de save conhecidas. Cada emulador vira um "jogo" com raízes de nome
//! fixo (`saves`, `states`, `memcards`…), iguais em todas as máquinas.

use crate::resolve::GameOs;
use std::path::{Path, PathBuf};

/// Uma pasta de save de emulador.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmuRoot {
    pub name: &'static str,
    /// Caminhos candidatos, relativos à pasta do usuário; o primeiro que existir vale.
    pub candidates: Vec<PathBuf>,
}

/// Um emulador.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emulator {
    pub id: &'static str,
    pub title: &'static str,
    pub roots: Vec<EmuRoot>,
    /// Nomes de executável, para o vigia de processos (fase 3).
    pub executables: &'static [&'static str],
}

fn r(name: &'static str, c: &[&str]) -> EmuRoot {
    EmuRoot {
        name,
        candidates: c.iter().map(PathBuf::from).collect(),
    }
}

/// Emuladores suportados, com os caminhos de cada sistema.
pub fn all(os: GameOs) -> Vec<Emulator> {
    use GameOs::*;
    let pick = |linux: Vec<EmuRoot>, windows: Vec<EmuRoot>, mac: Vec<EmuRoot>| match os {
        Linux => linux,
        Windows => windows,
        Mac => mac,
    };
    vec![
        Emulator {
            id: "retroarch",
            title: "RetroArch",
            executables: &["retroarch", "retroarch.exe"],
            roots: pick(
                vec![
                    r(
                        "saves",
                        &[
                            ".config/retroarch/saves",
                            ".var/app/org.libretro.RetroArch/config/retroarch/saves",
                        ],
                    ),
                    r(
                        "states",
                        &[
                            ".config/retroarch/states",
                            ".var/app/org.libretro.RetroArch/config/retroarch/states",
                        ],
                    ),
                ],
                vec![
                    r("saves", &["AppData/Roaming/RetroArch/saves"]),
                    r("states", &["AppData/Roaming/RetroArch/states"]),
                ],
                vec![
                    r("saves", &["Library/Application Support/RetroArch/saves"]),
                    r("states", &["Library/Application Support/RetroArch/states"]),
                ],
            ),
        },
        Emulator {
            id: "pcsx2",
            title: "PCSX2",
            executables: &["pcsx2", "pcsx2-qt", "pcsx2-qt.exe", "pcsx2.exe"],
            roots: pick(
                vec![
                    r(
                        "memcards",
                        &[
                            ".config/PCSX2/memcards",
                            ".var/app/net.pcsx2.PCSX2/config/PCSX2/memcards",
                        ],
                    ),
                    r(
                        "sstates",
                        &[
                            ".config/PCSX2/sstates",
                            ".var/app/net.pcsx2.PCSX2/config/PCSX2/sstates",
                        ],
                    ),
                ],
                vec![
                    r("memcards", &["Documents/PCSX2/memcards"]),
                    r("sstates", &["Documents/PCSX2/sstates"]),
                ],
                vec![
                    r("memcards", &["Library/Application Support/PCSX2/memcards"]),
                    r("sstates", &["Library/Application Support/PCSX2/sstates"]),
                ],
            ),
        },
        Emulator {
            id: "dolphin",
            title: "Dolphin",
            executables: &["dolphin-emu", "dolphin-emu-qt", "Dolphin.exe"],
            roots: pick(
                vec![
                    r(
                        "gc",
                        &[
                            ".local/share/dolphin-emu/GC",
                            ".dolphin-emu/GC",
                            ".var/app/org.DolphinEmu.dolphin-emu/data/dolphin-emu/GC",
                        ],
                    ),
                    r(
                        "wii",
                        &[
                            ".local/share/dolphin-emu/Wii",
                            ".dolphin-emu/Wii",
                            ".var/app/org.DolphinEmu.dolphin-emu/data/dolphin-emu/Wii",
                        ],
                    ),
                ],
                vec![
                    r(
                        "gc",
                        &[
                            "Documents/Dolphin Emulator/GC",
                            "AppData/Roaming/Dolphin Emulator/GC",
                        ],
                    ),
                    r(
                        "wii",
                        &[
                            "Documents/Dolphin Emulator/Wii",
                            "AppData/Roaming/Dolphin Emulator/Wii",
                        ],
                    ),
                ],
                vec![
                    r("gc", &["Library/Application Support/Dolphin/GC"]),
                    r("wii", &["Library/Application Support/Dolphin/Wii"]),
                ],
            ),
        },
        Emulator {
            id: "ryujinx",
            title: "Ryujinx",
            executables: &["Ryujinx", "Ryujinx.exe", "Ryujinx.Ava"],
            roots: pick(
                vec![r(
                    "save",
                    &[
                        ".config/Ryujinx/bis/user/save",
                        ".var/app/org.ryujinx.Ryujinx/config/Ryujinx/bis/user/save",
                    ],
                )],
                vec![r("save", &["AppData/Roaming/Ryujinx/bis/user/save"])],
                vec![r(
                    "save",
                    &["Library/Application Support/Ryujinx/bis/user/save"],
                )],
            ),
        },
        Emulator {
            id: "duckstation",
            title: "DuckStation",
            executables: &[
                "duckstation-qt",
                "duckstation-qt-x64-ReleaseLTCG.exe",
                "DuckStation",
            ],
            roots: pick(
                vec![
                    r(
                        "memcards",
                        &[
                            ".local/share/duckstation/memcards",
                            ".var/app/org.duckstation.DuckStation/data/duckstation/memcards",
                        ],
                    ),
                    r(
                        "savestates",
                        &[
                            ".local/share/duckstation/savestates",
                            ".var/app/org.duckstation.DuckStation/data/duckstation/savestates",
                        ],
                    ),
                ],
                vec![
                    r("memcards", &["Documents/DuckStation/memcards"]),
                    r("savestates", &["Documents/DuckStation/savestates"]),
                ],
                vec![
                    r(
                        "memcards",
                        &["Library/Application Support/DuckStation/memcards"],
                    ),
                    r(
                        "savestates",
                        &["Library/Application Support/DuckStation/savestates"],
                    ),
                ],
            ),
        },
        Emulator {
            id: "ppsspp",
            title: "PPSSPP",
            executables: &["PPSSPPSDL", "PPSSPPQt", "PPSSPPWindows64.exe", "ppsspp"],
            roots: pick(
                vec![
                    r(
                        "savedata",
                        &[
                            ".config/ppsspp/PSP/SAVEDATA",
                            ".var/app/org.ppsspp.PPSSPP/config/ppsspp/PSP/SAVEDATA",
                        ],
                    ),
                    r(
                        "states",
                        &[
                            ".config/ppsspp/PSP/PPSSPP_STATE",
                            ".var/app/org.ppsspp.PPSSPP/config/ppsspp/PSP/PPSSPP_STATE",
                        ],
                    ),
                ],
                vec![
                    r("savedata", &["Documents/PPSSPP/PSP/SAVEDATA"]),
                    r("states", &["Documents/PPSSPP/PSP/PPSSPP_STATE"]),
                ],
                vec![
                    r("savedata", &[".config/ppsspp/PSP/SAVEDATA"]),
                    r("states", &[".config/ppsspp/PSP/PPSSPP_STATE"]),
                ],
            ),
        },
    ]
}

/// Primeira pasta candidata que existe.
pub fn locate(home: &Path, root: &EmuRoot) -> Option<PathBuf> {
    root.candidates
        .iter()
        .map(|c| home.join(c))
        .find(|p| p.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todos_tem_raizes_nos_tres_sistemas() {
        for os in [GameOs::Linux, GameOs::Windows, GameOs::Mac] {
            let v = all(os);
            assert_eq!(v.len(), 6);
            assert!(v.iter().all(|e| !e.roots.is_empty()));
        }
        // Os nomes das raízes são iguais entre sistemas, para a sincronização casar.
        let names = |os| {
            all(os)
                .into_iter()
                .map(|e| e.roots.iter().map(|r| r.name).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(GameOs::Linux), names(GameOs::Windows));
        assert_eq!(names(GameOs::Linux), names(GameOs::Mac));
    }

    #[test]
    fn acha_flatpak() {
        let t = tempfile::tempdir().unwrap();
        let p = t
            .path()
            .join(".var/app/org.libretro.RetroArch/config/retroarch/saves");
        std::fs::create_dir_all(&p).unwrap();
        let ra = &all(GameOs::Linux)[0];
        assert_eq!(locate(t.path(), &ra.roots[0]), Some(p));
        assert_eq!(locate(t.path(), &ra.roots[1]), None);
    }
}
