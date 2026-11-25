//! Junta detectores, manifesto, emuladores e regras e devolve os jogos com save nesta máquina.

use crate::emulators;
use crate::manifest::{GameEntry, Manifest};
use crate::mirror;
use crate::resolve::{Env, GameOs, root_name, split_static};
use crate::rules::{self, Rule};
use crate::slug::slug;
use crate::stores::{self, Discovery, Install};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Opções do scan.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub home: PathBuf,
    /// Incluir arquivos marcados só como `config` no manifesto (desligado por padrão: costumam
    /// ser opções de vídeo da máquina).
    pub include_config: bool,
    /// Procurar todos os jogos do manifesto, não só os instalados por uma loja conhecida.
    pub search_all: bool,
    pub rules: Vec<Rule>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            home: relicario_platform::home_dir().unwrap_or_else(|| PathBuf::from("/")),
            include_config: false,
            search_all: true,
            rules: rules::builtin(),
        }
    }
}

/// Uma pasta de save encontrada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedRoot {
    /// Nome lógico, igual em todas as máquinas.
    pub name: String,
    pub path: PathBuf,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Outras pastas com o mesmo conteúdo, restauradas junto.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirrors: Vec<PathBuf>,
    pub files: u64,
    pub size: u64,
}

/// Um jogo com save nesta máquina.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedGame {
    /// Identificador estável (slug do título no manifesto).
    pub id: String,
    pub title: String,
    /// De onde veio: `manifest`, `emulator` ou `rule`.
    pub source: String,
    pub launcher: Option<String>,
    pub store: Option<String>,
    pub store_id: Option<String>,
    pub install_dir: Option<PathBuf>,
    pub prefix: Option<PathBuf>,
    /// O jogo tem Steam Cloud (segundo o manifesto).
    pub steam_cloud: bool,
    /// Executáveis conhecidos (para o vigia de processos).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub executables: Vec<String>,
    pub roots: Vec<DetectedRoot>,
}

impl DetectedGame {
    pub fn size(&self) -> u64 {
        self.roots.iter().map(|r| r.size).sum()
    }
}

/// Roda o scan completo.
pub fn scan(manifest: Option<&Manifest>, opts: &ScanOptions) -> Vec<DetectedGame> {
    let disc = stores::discover(&opts.home);
    scan_with(manifest, opts, &disc)
}

/// Scan com detecção já feita (útil em testes).
pub fn scan_with(
    manifest: Option<&Manifest>,
    opts: &ScanOptions,
    disc: &Discovery,
) -> Vec<DetectedGame> {
    let mut games: BTreeMap<String, DetectedGame> = BTreeMap::new();
    let native = {
        let mut e = Env::native();
        e.home = opts.home.clone();
        if e.os == GameOs::Linux {
            e.xdg_data = Some(
                std::env::var_os("XDG_DATA_HOME")
                    .map(PathBuf::from)
                    .filter(|_| opts.home == relicario_platform::home_dir().unwrap_or_default())
                    .unwrap_or_else(|| opts.home.join(".local/share")),
            );
            e.xdg_config = Some(
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .filter(|_| opts.home == relicario_platform::home_dir().unwrap_or_default())
                    .unwrap_or_else(|| opts.home.join(".config")),
            );
        }
        e
    };

    // 1. Jogos instalados por lojas conhecidas.
    if let Some(m) = manifest {
        for inst in &disc.installs {
            let Some((title, entry)) = find_entry(m, inst) else {
                continue;
            };
            let mut envs = Vec::new();
            let with_install = |e: Env| {
                let mut e =
                    e.with_install(inst.root.clone(), inst.dir_name.clone(), inst.base.clone());
                if let Some(s) = inst.store {
                    e = e.with_store(s);
                }
                e
            };
            if let Some(p) = &inst.prefix {
                envs.push(with_install(Env::wine(p)));
            }
            envs.push(with_install(native.clone()));
            let mut roots = Vec::new();
            for env in &envs {
                let mut env = env.clone();
                if inst.store_user_ids.len() == 1 {
                    env.store_user_id = Some(inst.store_user_ids[0].clone());
                }
                roots.extend(resolve_entry(entry, &env, opts.include_config, false));
            }
            add(&mut games, title, "manifest", Some(inst), entry, roots);
        }
    }

    // 2. Qualquer jogo do manifesto com save em lugares que não dependem da instalação.
    if opts.search_all
        && let Some(m) = manifest
    {
        let mut envs = vec![native.clone()];
        envs.extend(disc.shared_prefixes.iter().map(|p| Env::wine(p)));
        for (title, entry) in &m.games {
            if entry.alias.is_some() || entry.files.is_empty() {
                continue;
            }
            let mut roots = Vec::new();
            for env in &envs {
                roots.extend(resolve_entry(entry, env, opts.include_config, true));
            }
            if !roots.is_empty() {
                add(&mut games, title, "manifest", None, entry, roots);
            }
        }
    }

    // 3. Emuladores.
    for emu in emulators::all(native.os) {
        let mut roots = Vec::new();
        for r in &emu.roots {
            if let Some(p) = emulators::locate(&opts.home, r) {
                roots.push(Candidate {
                    name: r.name.to_string(),
                    template: r.name.to_string(),
                    dir: p,
                    include: vec!["**".into()],
                });
            }
        }
        let roots = finalize(roots);
        if !roots.is_empty() {
            games.insert(
                emu.id.to_string(),
                DetectedGame {
                    id: emu.id.to_string(),
                    title: emu.title.to_string(),
                    source: "emulator".into(),
                    launcher: None,
                    store: None,
                    store_id: None,
                    install_dir: None,
                    prefix: None,
                    steam_cloud: false,
                    executables: emu.executables.iter().map(|s| s.to_string()).collect(),
                    roots,
                },
            );
        }
    }

    // 4. Regras locais.
    for rule in &opts.rules {
        if rule.ignore {
            games.remove(&rule.game);
            continue;
        }
        if !rule.roots.is_empty() {
            let g = games
                .entry(rule.game.clone())
                .or_insert_with(|| DetectedGame {
                    id: rule.game.clone(),
                    title: rule.game.clone(),
                    source: "rule".into(),
                    launcher: None,
                    store: None,
                    store_id: None,
                    install_dir: None,
                    prefix: None,
                    steam_cloud: false,
                    executables: vec![],
                    roots: vec![],
                });
            for x in &rule.roots {
                let path = relicario_platform::expand_home(&x.path);
                g.roots.retain(|r| r.name != x.name);
                let include = if x.include.is_empty() {
                    vec!["**".to_string()]
                } else {
                    x.include.clone()
                };
                let (files, size) = count(&path, &include, &x.exclude);
                g.roots.push(DetectedRoot {
                    name: x.name.clone(),
                    path,
                    include,
                    exclude: x.exclude.clone(),
                    mirrors: vec![],
                    files,
                    size,
                });
            }
        }
        if let Some(g) = games.get_mut(&rule.game) {
            for r in &mut g.roots {
                for e in &rule.exclude {
                    if !r.exclude.contains(e) {
                        r.exclude.push(e.clone());
                    }
                }
                let (files, size) = count(&r.path, &r.include, &r.exclude);
                r.files = files;
                r.size = size;
            }
        }
    }

    // 5. Espelhos.
    for g in games.values_mut() {
        apply_mirrors(g);
        g.roots.retain(|r| r.files > 0 || g.source == "rule");
    }
    games.retain(|_, g| !g.roots.is_empty());
    games.into_values().collect()
}

fn find_entry<'a>(m: &'a Manifest, inst: &Install) -> Option<(&'a str, &'a GameEntry)> {
    let id = inst.id.as_deref();
    let by_id = match inst.store {
        Some("steam") => id
            .and_then(|i| i.parse().ok())
            .and_then(|i| m.by_steam_id(i)),
        Some("gog") => id.and_then(|i| i.parse().ok()).and_then(|i| m.by_gog_id(i)),
        _ if inst.launcher == "lutris" => id.and_then(|i| m.by_lutris(i)),
        _ => None,
    };
    by_id.or_else(|| m.by_name(&inst.title))
}

/// Raiz candidata antes de contar arquivos.
#[derive(Debug, Clone)]
struct Candidate {
    name: String,
    template: String,
    dir: PathBuf,
    include: Vec<String>,
}

fn resolve_entry(
    entry: &GameEntry,
    env: &Env,
    include_config: bool,
    lenient_store: bool,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (pattern, rule) in &entry.files {
        let only_config =
            rule.tags.iter().any(|t| t == "config") && !rule.tags.iter().any(|t| t == "save");
        if only_config && !include_config {
            continue;
        }
        let applies = if lenient_store {
            rule.when.is_empty()
                || rule.when.iter().any(|c| {
                    let mut c = c.clone();
                    c.store = None;
                    env.matches(&c)
                })
        } else {
            env.applies(rule)
        };
        if !applies {
            continue;
        }
        // Sem instalação, padrões que dependem dela não valem.
        if lenient_store
            && (pattern.contains("<base>")
                || pattern.contains("<root>")
                || pattern.contains("<game>"))
        {
            continue;
        }
        let Some(resolved) = env.resolve(pattern) else {
            continue;
        };
        let (dir, rest) = split_static(&resolved);
        let include = match rest {
            Some(r) => vec![r.clone(), format!("{r}/**")],
            None => {
                if dir.is_file() {
                    let name = dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let parent = dir.parent().map(Path::to_path_buf).unwrap_or_default();
                    out.push(Candidate {
                        name: String::new(),
                        template: pattern.clone(),
                        dir: parent,
                        include: vec![escape(&name)],
                    });
                    continue;
                }
                vec!["**".into()]
            }
        };
        if !dir.is_dir() || dir.parent().is_none() || dir == env.home {
            // Nunca a pasta do usuário inteira nem a raiz do disco.
            continue;
        }
        out.push(Candidate {
            name: String::new(),
            template: pattern.clone(),
            dir,
            include,
        });
    }
    out
}

fn escape(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}') {
            s.push('[');
            s.push(c);
            s.push(']');
        } else {
            s.push(c);
        }
    }
    s
}

/// Junta candidatas da mesma pasta, dá nomes únicos e conta arquivos.
fn finalize(mut cands: Vec<Candidate>) -> Vec<DetectedRoot> {
    // Mesma pasta: une os padrões.
    let mut by_dir: Vec<Candidate> = Vec::new();
    cands.sort_by(|a, b| a.dir.cmp(&b.dir).then(a.template.cmp(&b.template)));
    for c in cands {
        if let Some(prev) = by_dir.iter_mut().find(|p| p.dir == c.dir) {
            for i in c.include {
                if !prev.include.contains(&i) {
                    prev.include.push(i);
                }
            }
            continue;
        }
        by_dir.push(c);
    }
    // Pasta dentro de outra que já pega tudo: descarta.
    let all: Vec<(PathBuf, bool)> = by_dir
        .iter()
        .map(|c| (c.dir.clone(), c.include.iter().any(|i| i == "**")))
        .collect();
    by_dir.retain(|c| {
        !all.iter()
            .any(|(d, everything)| *everything && d != &c.dir && c.dir.starts_with(d))
    });
    // Nomes únicos: mais partes do caminho até desempatar.
    for depth in 1..6 {
        let names: Vec<String> = by_dir
            .iter()
            .map(|c| {
                if c.name.is_empty() || depth > 1 {
                    root_name(&c.template, depth)
                } else {
                    c.name.clone()
                }
            })
            .collect();
        let unique = names
            .iter()
            .all(|n| names.iter().filter(|m| *m == n).count() == 1);
        if unique || depth == 5 {
            for (c, n) in by_dir.iter_mut().zip(names) {
                c.name = n;
            }
            break;
        }
    }
    let mut seen = std::collections::HashMap::<String, usize>::new();
    let mut out = Vec::new();
    for c in by_dir {
        let k = seen.entry(c.name.clone()).or_insert(0);
        *k += 1;
        let name = if *k > 1 {
            format!("{}-{}", c.name, k)
        } else {
            c.name
        };
        let (files, size) = count(&c.dir, &c.include, &[]);
        out.push(DetectedRoot {
            name,
            path: c.dir,
            include: c.include,
            exclude: vec![],
            mirrors: vec![],
            files,
            size,
        });
    }
    out
}

fn add(
    games: &mut BTreeMap<String, DetectedGame>,
    title: &str,
    source: &str,
    inst: Option<&Install>,
    entry: &GameEntry,
    roots: Vec<Candidate>,
) {
    let roots = finalize(roots);
    let id = slug(title);
    let executables = entry
        .launch
        .keys()
        .filter_map(|k| k.rsplit('/').next())
        .filter(|n| !n.is_empty() && !n.contains('<') && !n.contains('*'))
        .map(str::to_string)
        .collect();
    let g = games.entry(id.clone()).or_insert_with(|| DetectedGame {
        id,
        title: title.to_string(),
        source: source.to_string(),
        launcher: inst.map(|i| i.launcher.to_string()),
        store: inst.and_then(|i| i.store.map(str::to_string)),
        store_id: inst.and_then(|i| i.id.clone()),
        install_dir: inst.and_then(|i| i.base.clone()),
        prefix: inst.and_then(|i| i.prefix.clone()),
        steam_cloud: entry.steam_cloud(),
        executables,
        roots: vec![],
    });
    if g.launcher.is_none()
        && let Some(i) = inst
    {
        g.launcher = Some(i.launcher.to_string());
        g.store = i.store.map(str::to_string);
        g.store_id = i.id.clone();
        g.install_dir = i.base.clone();
        g.prefix = i.prefix.clone();
    }
    for r in roots {
        if !g.roots.iter().any(|x| x.path == r.path) {
            let mut r = r;
            if g.roots.iter().any(|x| x.name == r.name) {
                r.name = format!("{}-{}", r.name, g.roots.len() + 1);
            }
            g.roots.push(r);
        }
    }
}

fn globset(pats: &[String]) -> Option<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in pats {
        b.add(GlobBuilder::new(p).literal_separator(true).build().ok()?);
    }
    b.build().ok()
}

/// Arquivos de uma raiz (relativo, absoluto) que passam nos filtros.
pub fn list_files(dir: &Path, include: &[String], exclude: &[String]) -> Vec<(String, PathBuf)> {
    let (Some(inc), Some(exc)) = (globset(include), globset(exclude)) else {
        return vec![];
    };
    let mut out = Vec::new();
    walk(
        dir,
        "",
        &mut |rel, p| {
            if inc.is_match(rel) && !exc.is_match(rel) {
                out.push((rel.to_string(), p.to_path_buf()));
            }
        },
        0,
    );
    out.sort();
    out
}

fn walk(dir: &Path, prefix: &str, f: &mut dyn FnMut(&str, &Path), depth: usize) {
    if depth > 24 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.filter_map(|e| e.ok()) {
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        let Ok(ft) = e.file_type() else { continue };
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if ft.is_dir() {
            walk(&e.path(), &rel, f, depth + 1);
        } else if ft.is_file() {
            f(&rel, &e.path());
        }
    }
}

fn count(dir: &Path, include: &[String], exclude: &[String]) -> (u64, u64) {
    let files = list_files(dir, include, exclude);
    let size = files
        .iter()
        .map(|(_, p)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum();
    (files.len() as u64, size)
}

fn apply_mirrors(g: &mut DetectedGame) {
    if g.roots.len() < 2 {
        return;
    }
    let listed: Vec<(PathBuf, Vec<(String, PathBuf)>)> = g
        .roots
        .iter()
        .map(|r| (r.path.clone(), list_files(&r.path, &r.include, &r.exclude)))
        .collect();
    let pairs = mirror::detect(&listed);
    let mut remove = Vec::new();
    for (p, m) in pairs {
        let mp = g.roots[m].path.clone();
        g.roots[p].mirrors.push(mp);
        remove.push(m);
    }
    remove.sort_unstable();
    for i in remove.into_iter().rev() {
        g.roots.remove(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const YAML: &str = r#"
Hookshot:
  cloud:
    steam: true
  files:
    "<home>/.config/unity3d/NanningsGames/Hookshot/*.sav":
      when:
        - os: linux
          store: steam
    "<home>/AppData/LocalLow/NanningsGames/Hookshot/*.sav":
      when:
        - store: steam
    "<home>/.config/unity3d/NanningsGames/Hookshot/prefs":
      tags:
        - config
  installDir:
    Hookshot: {}
  launch:
    "<base>/Hookshot.x86_64":
      - when:
          - os: linux
  steam:
    id: 1031570
Baldur's Gate 3:
  files:
    "<winLocalAppData>/Larian Studios/Baldur's Gate 3/PlayerProfiles":
      tags:
        - save
      when:
        - os: windows
    "<root>/userdata/<storeUserId>/1086940/remote":
      tags:
        - save
      when:
        - store: steam
  steam:
    id: 1086940
Jogo Solto:
  files:
    "<xdgData>/jogo-solto/saves":
      tags:
        - save
      when:
        - os: linux
    "<winDocuments>/Jogo Solto":
      when:
        - os: windows
"#;

    #[test]
    fn scan_completo_em_maquina_falsa() {
        if GameOs::host() != GameOs::Linux {
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let steam = crate::stores::steam::tests::fake_steam(home);
        // Hookshot nativo.
        let hs = home.join(".config/unity3d/NanningsGames/Hookshot");
        fs::create_dir_all(hs.join("save-backups")).unwrap();
        fs::write(hs.join("caption.sav"), b"c1").unwrap();
        fs::write(hs.join("prefs"), b"vsync=1").unwrap();
        // BG3 no Proton, espelhado no userdata.
        let pfx = home.join("Games/SteamLibrary/steamapps/compatdata/1086940/pfx");
        let bg3 = pfx.join(
            "drive_c/users/steamuser/AppData/Local/Larian Studios/Baldur's Gate 3/PlayerProfiles",
        );
        fs::create_dir_all(bg3.join("Public/Savegames")).unwrap();
        fs::write(bg3.join("Public/Savegames/slot.lsv"), b"bg3").unwrap();
        let remote = steam.join("userdata/12345/1086940/remote");
        fs::create_dir_all(remote.join("Public/Savegames")).unwrap();
        fs::write(remote.join("Public/Savegames/slot.lsv"), b"bg3").unwrap();
        // Jogo sem loja, encontrado pelo manifesto.
        fs::create_dir_all(home.join(".local/share/jogo-solto/saves")).unwrap();
        fs::write(home.join(".local/share/jogo-solto/saves/1.dat"), b"x").unwrap();
        // RetroArch.
        fs::create_dir_all(home.join(".config/retroarch/saves")).unwrap();
        fs::write(home.join(".config/retroarch/saves/zelda.srm"), b"z").unwrap();

        let m = Manifest::from_yaml(YAML).unwrap();
        let opts = ScanOptions {
            home: home.to_path_buf(),
            ..ScanOptions::default()
        };
        let start = std::time::Instant::now();
        let games = scan(Some(&m), &opts);
        assert!(start.elapsed().as_secs() < 5);
        let ids: Vec<&str> = games.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["baldurs-gate-3", "hookshot", "jogo-solto", "retroarch"],
            "{games:#?}"
        );

        let hook = games.iter().find(|g| g.id == "hookshot").unwrap();
        assert_eq!(hook.launcher.as_deref(), Some("steam"));
        assert!(hook.steam_cloud);
        assert_eq!(hook.executables, vec!["Hookshot.x86_64".to_string()]);
        assert_eq!(
            hook.roots.len(),
            1,
            "config fica de fora: {:#?}",
            hook.roots
        );
        assert_eq!(hook.roots[0].name, "hookshot");
        assert_eq!(hook.roots[0].files, 1);
        assert!(
            hook.roots[0]
                .exclude
                .iter()
                .any(|e| e.contains("hookshot_options.ini"))
        );

        let b = games.iter().find(|g| g.id == "baldurs-gate-3").unwrap();
        assert_eq!(b.roots.len(), 1, "espelho unido: {:#?}", b.roots);
        assert_eq!(b.roots[0].path, bg3);
        assert_eq!(b.roots[0].mirrors, vec![remote.clone()]);
        assert_eq!(b.prefix.as_deref(), Some(pfx.as_path()));

        // Espelho só quando idêntico: mudar o remote desfaz.
        fs::write(remote.join("Public/Savegames/slot.lsv"), b"bg3-outro").unwrap();
        let games = scan(Some(&m), &opts);
        let b = games.iter().find(|g| g.id == "baldurs-gate-3").unwrap();
        assert_eq!(b.roots.len(), 2);
        assert!(b.roots.iter().all(|r| r.mirrors.is_empty()));
        let names: Vec<&str> = b.roots.iter().map(|r| r.name.as_str()).collect();
        assert!(
            names.contains(&"playerprofiles") && names.contains(&"remote"),
            "{names:?}"
        );

        // Regra do usuário: ignorar um jogo.
        let mut opts2 = opts.clone();
        opts2.rules.push(Rule {
            game: "jogo-solto".into(),
            ignore: true,
            ..Rule::default()
        });
        assert!(!scan(Some(&m), &opts2).iter().any(|g| g.id == "jogo-solto"));

        // Sem manifesto, só emuladores e regras.
        let only = scan(None, &opts);
        assert_eq!(
            only.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
            vec!["retroarch"]
        );
    }
}
