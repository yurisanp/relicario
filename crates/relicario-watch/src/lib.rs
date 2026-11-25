//! Vigia de processos e de arquivos.
//!
//! - [`procs`]: lista de processos (no Linux lendo `/proc` direto, que é barato; nos outros
//!   sistemas via `sysinfo`).
//! - [`Matcher`] e [`Watcher`]: quais processos são jogos conhecidos, e eventos de abriu/fechou.
//! - [`settle`]: esperar o jogo terminar de gravar.

pub mod procs;
pub mod settle;

pub use procs::ProcInfo;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Como reconhecer um jogo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameMatch {
    pub id: String,
    /// Nomes de executável (comparação sem diferenciar maiúsculas).
    pub exe_names: Vec<String>,
    /// Caminhos completos de executável associados pelo usuário.
    pub exe_paths: Vec<PathBuf>,
    /// Pastas de instalação: qualquer executável dentro delas conta.
    pub install_dirs: Vec<PathBuf>,
}

/// Processos que nunca são o jogo em si.
const IGNORED: &[&str] = &[
    "steam",
    "steam.exe",
    "steamwebhelper",
    "steamwebhelper.exe",
    "steamerrorreporter",
    "steamerrorreporter.exe",
    "wineserver",
    "wine",
    "wine64",
    "wine-preloader",
    "wine64-preloader",
    "explorer.exe",
    "services.exe",
    "winedevice.exe",
    "plugplay.exe",
    "svchost.exe",
    "rpcss.exe",
    "tabtip.exe",
    "conhost.exe",
    "start.exe",
    "reaper",
    "pressure-vessel-wrap",
    "pv-bwrap",
    "pv-adverb",
    "srt-bwrap",
    "proton",
    "python3",
    "python",
    "bash",
    "sh",
    "heroic",
    "lutris",
    "legendary",
    "gogdl",
    "unitycrashhandler64.exe",
    "crashpad_handler",
    "unitycrashhandler32.exe",
    "crashreportclient.exe",
    "relicario",
    "relicario.exe",
    "relicario-daemon",
];

/// Reconhece jogos entre os processos.
#[derive(Debug, Clone, Default)]
pub struct Matcher {
    games: Vec<GameMatch>,
}

fn basename(s: &str) -> &str {
    s.rsplit(['/', '\\']).next().unwrap_or(s)
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

impl Matcher {
    pub fn new(games: Vec<GameMatch>) -> Self {
        let games = games
            .into_iter()
            .map(|mut g| {
                g.exe_names = g.exe_names.iter().map(|n| lower(basename(n))).collect();
                g
            })
            .collect();
        Matcher { games }
    }

    pub fn is_ignored(p: &ProcInfo) -> bool {
        let n = lower(&p.name);
        IGNORED.contains(&n.as_str())
            || p.exe
                .as_ref()
                .and_then(|e| e.file_name())
                .is_some_and(|f| IGNORED.contains(&lower(&f.to_string_lossy()).as_str()))
    }

    /// Nomes que identificam o processo: nome, executável e, para Wine/Proton, o `.exe` da
    /// linha de comando.
    fn names(p: &ProcInfo) -> Vec<String> {
        let mut v = vec![lower(&p.name)];
        if let Some(e) = &p.exe
            && let Some(f) = e.file_name()
        {
            v.push(lower(&f.to_string_lossy()));
        }
        for a in p.cmdline.iter().take(4) {
            let b = lower(basename(a));
            if !b.is_empty() {
                v.push(b);
            }
        }
        v
    }

    fn paths(p: &ProcInfo) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = p.exe.iter().cloned().collect();
        for a in p.cmdline.iter().take(4) {
            // Caminho Windows de um jogo no Proton (Z:\home\...\Jogo.exe).
            let unix = a
                .strip_prefix("Z:")
                .or_else(|| a.strip_prefix("z:"))
                .map(|r| r.replace('\\', "/"));
            v.push(PathBuf::from(unix.unwrap_or_else(|| a.clone())));
        }
        v
    }

    /// Id do jogo deste processo, se for um.
    pub fn match_proc(&self, p: &ProcInfo) -> Option<&str> {
        if Self::is_ignored(p) {
            return None;
        }
        let names = Self::names(p);
        let paths = Self::paths(p);
        for g in &self.games {
            if g.exe_paths
                .iter()
                .any(|ep| paths.iter().any(|pp| same_path(ep, pp)))
            {
                return Some(&g.id);
            }
            if g.exe_names.iter().any(|n| names.contains(n)) {
                return Some(&g.id);
            }
            if g.install_dirs.iter().any(|d| {
                paths
                    .iter()
                    .any(|pp| pp.starts_with(d) && is_executable_like(pp))
            }) {
                return Some(&g.id);
            }
        }
        None
    }

    /// Processos que parecem jogos mas não são reconhecidos: executável dentro de uma pasta de
    /// biblioteca de jogos ou um `.exe` rodando no Wine.
    pub fn unknown_candidates<'a>(
        &self,
        procs: &'a [ProcInfo],
        libraries: &[PathBuf],
    ) -> Vec<&'a ProcInfo> {
        procs
            .iter()
            .filter(|p| !Self::is_ignored(p) && self.match_proc(p).is_none())
            .filter(|p| {
                let paths = Self::paths(p);
                let in_lib = paths.iter().any(|pp| {
                    libraries.iter().any(|l| pp.starts_with(l)) && is_executable_like(pp)
                });
                let wine_exe = p
                    .cmdline
                    .first()
                    .is_some_and(|a| lower(a).ends_with(".exe"))
                    && !cfg!(windows);
                in_lib || wine_exe
            })
            .collect()
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        lower(&a.to_string_lossy().replace('\\', "/"))
            == lower(&b.to_string_lossy().replace('\\', "/"))
    } else {
        a == b
    }
}

fn is_executable_like(p: &Path) -> bool {
    let name = lower(
        &p.file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    !(name.ends_with(".so") || name.ends_with(".dll") || name.ends_with(".sh") || name.is_empty())
}

/// Abriu ou fechou.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Started { game: String, pid: u32 },
    Stopped { game: String },
}

/// Compara listas sucessivas de processos e emite eventos por jogo.
#[derive(Debug, Default)]
pub struct Watcher {
    pub matcher: Matcher,
    running: BTreeMap<String, BTreeSet<u32>>,
}

impl Watcher {
    pub fn new(matcher: Matcher) -> Self {
        Watcher {
            matcher,
            running: BTreeMap::new(),
        }
    }

    pub fn set_matcher(&mut self, m: Matcher) {
        self.matcher = m;
    }

    pub fn running(&self) -> impl Iterator<Item = &String> {
        self.running.keys()
    }

    pub fn is_running(&self, game: &str) -> bool {
        self.running.contains_key(game)
    }

    /// Processa uma lista de processos e devolve o que mudou.
    pub fn update(&mut self, procs: &[ProcInfo]) -> Vec<Event> {
        let mut now: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
        for p in procs {
            if let Some(g) = self.matcher.match_proc(p) {
                now.entry(g.to_string()).or_default().insert(p.pid);
            }
        }
        let mut ev = Vec::new();
        for (g, pids) in &now {
            if !self.running.contains_key(g) {
                ev.push(Event::Started {
                    game: g.clone(),
                    pid: *pids.iter().next().expect("pid"),
                });
            }
        }
        for g in self.running.keys() {
            if !now.contains_key(g) {
                ev.push(Event::Stopped { game: g.clone() });
            }
        }
        self.running = now;
        ev
    }

    /// Lê os processos do sistema e atualiza.
    pub fn poll(&mut self) -> Vec<Event> {
        let procs = procs::list();
        self.update(&procs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, name: &str, exe: Option<&str>, cmd: &[&str]) -> ProcInfo {
        ProcInfo {
            pid,
            name: name.into(),
            exe: exe.map(PathBuf::from),
            cmdline: cmd.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn reconhece_nativo_proton_e_pasta() {
        let m = Matcher::new(vec![
            GameMatch {
                id: "hookshot".into(),
                exe_names: vec!["<base>/Hookshot.x86_64".into()],
                ..Default::default()
            },
            GameMatch {
                id: "bg3".into(),
                install_dirs: vec!["/games/common/Baldurs Gate 3".into()],
                ..Default::default()
            },
            GameMatch {
                id: "solto".into(),
                exe_paths: vec!["/home/y/jogos/solto/run".into()],
                ..Default::default()
            },
        ]);
        assert_eq!(
            m.match_proc(&p(1, "Hookshot.x86_64", Some("/g/Hookshot.x86_64"), &[])),
            Some("hookshot")
        );
        assert_eq!(
            m.match_proc(&p(
                2,
                "bg3.exe",
                None,
                &["Z:\\games\\common\\Baldurs Gate 3\\bin\\bg3.exe"]
            )),
            Some("bg3")
        );
        assert_eq!(
            m.match_proc(&p(3, "run", Some("/home/y/jogos/solto/run"), &[])),
            Some("solto")
        );
        assert_eq!(
            m.match_proc(&p(
                4,
                "steam",
                Some("/games/common/Baldurs Gate 3/steam"),
                &[]
            )),
            None,
            "ignorado"
        );
        assert_eq!(
            m.match_proc(&p(5, "firefox", Some("/usr/bin/firefox"), &[])),
            None
        );
        let procs = vec![
            p(
                6,
                "Desconhecido.exe",
                None,
                &["C:\\Jogos\\Desconhecido.exe"],
            ),
            p(7, "outro", Some("/games/library/Outro/outro"), &[]),
            p(8, "firefox", Some("/usr/bin/firefox"), &[]),
        ];
        let c = m.unknown_candidates(&procs, &["/games/library".into()]);
        let pids: Vec<u32> = c.iter().map(|p| p.pid).collect();
        if cfg!(windows) {
            assert_eq!(pids, vec![7]);
        } else {
            assert_eq!(pids, vec![6, 7]);
        }
    }

    #[test]
    fn eventos() {
        let mut w = Watcher::new(Matcher::new(vec![GameMatch {
            id: "a".into(),
            exe_names: vec!["a.exe".into()],
            ..Default::default()
        }]));
        assert!(w.update(&[]).is_empty());
        let e = w.update(&[p(10, "a.exe", None, &[]), p(11, "a.exe", None, &[])]);
        assert_eq!(
            e,
            vec![Event::Started {
                game: "a".into(),
                pid: 10
            }]
        );
        assert!(w.is_running("a"));
        assert!(
            w.update(&[p(11, "a.exe", None, &[])]).is_empty(),
            "ainda tem um processo"
        );
        assert_eq!(w.update(&[]), vec![Event::Stopped { game: "a".into() }]);
    }

    #[test]
    fn lista_processos_do_sistema_inclui_este() {
        let me = std::process::id();
        assert!(procs::list().iter().any(|p| p.pid == me));
    }
}
