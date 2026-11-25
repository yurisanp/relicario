//! Importação do Ludusavi: backups (formato simples e zip, completos e diferenciais) e jogos
//! personalizados.
//!
//! Cada backup vira um snapshot de histórico (motivo `Import`) desta máquina, em ordem, com o
//! horário original. Nada é restaurado nem vira head: o que já está no disco continua mandando.
//! O backup mais recente de cada jogo fica fixado, para a retenção não liberar.
//! Cada arquivo é conferido pelo SHA-1 do `mapping.yaml` antes de entrar.

use crate::config::{Config, GameConfig, RootConfig};
use crate::games::Game;
use relicario_model::{GameId, SnapshotId};
use relicario_store::ops::{self, Local};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct Mapping {
    name: String,
    #[serde(default)]
    drives: BTreeMap<String, String>,
    #[serde(default)]
    backups: Vec<Full>,
}

#[derive(Debug, Deserialize)]
struct Full {
    name: String,
    when: Option<String>,
    #[serde(default)]
    files: BTreeMap<String, MFile>,
    #[serde(default)]
    children: Vec<Diff>,
}

#[derive(Debug, Deserialize)]
struct Diff {
    name: String,
    when: Option<String>,
    #[serde(default)]
    files: BTreeMap<String, Option<MFile>>,
}

#[derive(Debug, Clone, Deserialize)]
struct MFile {
    hash: String,
    #[serde(default)]
    size: u64,
}

/// Estado dos arquivos num backup: caminho original → (dados do mapping, arquivo no backup).
type Files = BTreeMap<String, (MFile, PathBuf)>;
/// Um ponto da cadeia: (nome do backup, quando, arquivos).
type Step = (String, Option<String>, Files);

/// Resultado por jogo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameImport {
    pub name: String,
    pub id: String,
    pub snapshots: usize,
    pub files: usize,
    pub bytes: u64,
    /// Arquivos que não bateram com o hash ou faltavam.
    pub problems: Vec<String>,
    /// Arquivos sem pasta correspondente neste jogo (guardados na raiz `ludusavi`).
    pub unmapped: usize,
}

/// `2024-01-02T03:04:05.678Z` ou com fuso (`-03:00`) → milissegundos desde 1970.
pub fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once(['T', ' '])?;
    let mut d = date.split('-');
    let (y, m, dd): (i64, i64, i64) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    let (time, tz) = match rest.find(['Z', 'z', '+', '-']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "Z"),
    };
    let mut t = time.split(':');
    let h: i64 = t.next()?.parse().ok()?;
    let mi: i64 = t.next()?.parse().ok()?;
    let sec: f64 = t.next().unwrap_or("0").parse().ok()?;
    let off = match tz {
        "Z" | "z" => 0,
        _ => {
            let sign = if tz.starts_with('-') { -1 } else { 1 };
            let (oh, om) = tz[1..]
                .split_once(':')
                .unwrap_or((&tz[1..3.min(tz.len())], "0"));
            sign * (oh.parse::<i64>().ok()? * 60 + om.parse::<i64>().ok()?)
        }
    };
    // Dias desde 1970 (algoritmo de Howard Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(
        ((days * 86400 + h * 3600 + mi * 60 - off * 60) as f64 * 1000.0 + sec * 1000.0).round()
            as i64,
    )
}

/// Pasta e caminho relativo de um arquivo dentro do backup.
fn backup_rel(drives: &BTreeMap<String, String>, original: &str) -> Option<String> {
    let norm = original.replace('\\', "/");
    // Drive mais longo que casa primeiro (ex.: "C:" antes de "").
    let mut ds: Vec<(&String, &String)> = drives.iter().collect();
    ds.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    for (folder, prefix) in ds {
        if let Some(rest) = norm.strip_prefix(prefix.as_str()) {
            return Some(format!("{folder}/{}", rest.trim_start_matches('/')));
        }
    }
    None
}

fn sha1_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha1::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Onde o conteúdo de um backup está: a pasta, ou um zip extraído num temporário.
fn backup_dir(game_dir: &Path, name: &str, tmp: &Path) -> Result<PathBuf, String> {
    if name == "." {
        return Ok(game_dir.to_path_buf());
    }
    let p = game_dir.join(name);
    if p.is_dir() {
        return Ok(p);
    }
    let zip = if p.is_file() {
        p.clone()
    } else {
        game_dir.join(format!("{name}.zip"))
    };
    if zip.is_file() {
        let out = tmp.join(name.trim_end_matches(".zip"));
        let f = std::fs::File::open(&zip).map_err(|e| format!("{}: {e}", zip.display()))?;
        let mut a = zip::ZipArchive::new(f).map_err(|e| format!("{}: {e}", zip.display()))?;
        a.extract(&out)
            .map_err(|e| format!("{}: {e}", zip.display()))?;
        return Ok(out);
    }
    Err(format!(
        "backup {name} não encontrado em {}",
        game_dir.display()
    ))
}

/// Raiz do jogo onde um caminho original cai: (raiz, relativo). Sem raiz, `ludusavi` com o
/// caminho inteiro.
fn place(roots: &[(String, PathBuf)], original: &str) -> (String, String, bool) {
    let norm = original.replace('\\', "/");
    for (name, path) in roots {
        let base = path.to_string_lossy().replace('\\', "/");
        let base = base.trim_end_matches('/');
        if let Some(rest) = norm.strip_prefix(base)
            && let Some(rest) = rest.strip_prefix('/')
        {
            return (name.clone(), rest.to_string(), true);
        }
    }
    let rel = norm.trim_start_matches('/').replace(':', "");
    ("ludusavi".into(), rel, false)
}

fn find_game<'a>(all: &'a [Game], name: &str) -> Option<&'a Game> {
    let s = relicario_catalog::slug::slug(name);
    all.iter().find(|g| {
        g.title().eq_ignore_ascii_case(name)
            || g.id() == s
            || relicario_catalog::slug::slug(g.title()) == s
    })
}

/// Importa um jogo do Ludusavi (pasta com `mapping.yaml`).
pub fn import_game(l: &mut Local<'_>, all: &[Game], game_dir: &Path) -> Result<GameImport, String> {
    let text = std::fs::read_to_string(game_dir.join("mapping.yaml"))
        .map_err(|e| format!("{}: {e}", game_dir.display()))?;
    let m: Mapping = serde_yaml_ng::from_str(&text)
        .map_err(|e| format!("{}/mapping.yaml: {e}", game_dir.display()))?;
    let game = find_game(all, &m.name);
    let id = game
        .map(|g| g.id().to_string())
        .unwrap_or_else(|| relicario_catalog::slug::slug(&m.name));
    let roots: Vec<(String, PathBuf)> = game
        .map(|g| {
            g.config
                .roots
                .iter()
                .map(|r| (r.name.clone(), relicario_platform::expand_home(&r.path)))
                .collect()
        })
        .unwrap_or_default();
    let mut out = GameImport {
        name: m.name.clone(),
        id: id.clone(),
        ..Default::default()
    };
    let tmp = std::env::temp_dir().join(format!(
        "relicario-ludusavi-{}-{}",
        std::process::id(),
        relicario_catalog::slug::slug(&m.name)
    ));
    let gid = GameId::new(&id);
    let mut last: Option<SnapshotId> = None;
    for full in &m.backups {
        let mut state: Files = BTreeMap::new();
        let dir = backup_dir(game_dir, &full.name, &tmp)?;
        for (orig, f) in &full.files {
            if let Some(rel) = backup_rel(&m.drives, orig) {
                state.insert(orig.clone(), (f.clone(), dir.join(rel)));
            } else {
                out.problems.push(format!("{orig}: drive desconhecido"));
            }
        }
        let mut steps: Vec<Step> = vec![(full.name.clone(), full.when.clone(), state.clone())];
        for d in &full.children {
            let ddir = backup_dir(game_dir, &d.name, &tmp)?;
            for (orig, f) in &d.files {
                match f {
                    Some(f) => match backup_rel(&m.drives, orig) {
                        Some(rel) => {
                            state.insert(orig.clone(), (f.clone(), ddir.join(rel)));
                        }
                        None => out.problems.push(format!("{orig}: drive desconhecido")),
                    },
                    None => {
                        state.remove(orig);
                    }
                }
            }
            steps.push((d.name.clone(), d.when.clone(), state.clone()));
        }
        for (name, when, files) in steps {
            let mut entries = Vec::new();
            for (orig, (f, path)) in &files {
                match sha1_file(path) {
                    Ok(h) if h.eq_ignore_ascii_case(&f.hash) => {
                        let (root, rel, mapped) = place(&roots, orig);
                        if !mapped {
                            out.unmapped += 1;
                        }
                        out.bytes += f.size;
                        entries.push((root, rel, path.clone()));
                    }
                    Ok(_) => out
                        .problems
                        .push(format!("{name}: {orig} não confere com o SHA-1 do mapping")),
                    Err(e) => out.problems.push(format!("{name}: {orig}: {e}")),
                }
            }
            out.files += entries.len();
            let tree = relicario_store::files::store_entries(l.repo, &entries)
                .map_err(|e| e.to_string())?;
            let mut meta = BTreeMap::new();
            meta.insert("ludusavi.backup".to_string(), name.clone());
            meta.insert("ludusavi.game".to_string(), m.name.clone());
            let at = when
                .as_deref()
                .and_then(parse_time)
                .unwrap_or_else(relicario_store::time::now);
            let sid = ops::import_snapshot(l, &gid, &tree, last.into_iter().collect(), at, meta)
                .map_err(|e| e.to_string())?;
            last = Some(sid);
            out.snapshots += 1;
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    if let Some(s) = last {
        ops::pin(l, s, "importado do Ludusavi".into()).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Importa todos os jogos de uma pasta de backups do Ludusavi.
pub fn import_backups(
    cfg: &Config,
    all: &[Game],
    root: &Path,
    only: Option<&str>,
) -> Result<Vec<Result<GameImport, String>>, String> {
    let mut repo = cfg.open_repo()?;
    let mut l = Local {
        repo: &mut repo,
        machine: cfg.machine(),
        machine_name: cfg.machine_name.clone(),
    };
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .map_err(|e| format!("{}: {e}", root.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join("mapping.yaml").is_file())
        .collect();
    dirs.sort();
    if dirs.is_empty() {
        return Err(format!(
            "nenhum backup do Ludusavi em {} (falta mapping.yaml)",
            root.display()
        ));
    }
    let mut out = Vec::new();
    for d in dirs {
        if let Some(o) = only {
            let name = d
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !name.contains(&o.to_lowercase()) {
                continue;
            }
        }
        out.push(import_game(&mut l, all, &d));
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LudusaviConfig {
    #[serde(default)]
    custom_games: Vec<CustomGame>,
    #[serde(default)]
    backup: Option<BackupCfg>,
}

#[derive(Debug, Deserialize)]
struct BackupCfg {
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CustomGame {
    name: String,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    ignore: bool,
}

/// Onde o Ludusavi guarda a configuração nesta máquina.
pub fn default_config_path() -> Option<PathBuf> {
    let home = relicario_platform::home_dir()?;
    let cands = [
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("ludusavi/config.yaml")),
        Some(home.join(".config/ludusavi/config.yaml")),
        Some(home.join(".var/app/com.mtkennerly.ludusavi/config/ludusavi/config.yaml")),
        Some(home.join("Library/Application Support/ludusavi/config.yaml")),
    ];
    cands.into_iter().flatten().find(|p| p.is_file())
}

/// Pasta de backups configurada no Ludusavi.
pub fn backup_path(config: &Path) -> Option<PathBuf> {
    let c: LudusaviConfig = serde_yaml_ng::from_str(&std::fs::read_to_string(config).ok()?).ok()?;
    c.backup
        .and_then(|b| b.path)
        .map(|p| relicario_platform::expand_home(&p))
}

/// Jogos personalizados do Ludusavi como jogos do Relicário. Caminhos com marcadores que não
/// dá para resolver aqui (`<base>`, `<storeUserId>`...) são pulados e listados.
pub fn custom_games(config: &Path) -> Result<(Vec<GameConfig>, Vec<String>), String> {
    let text = std::fs::read_to_string(config).map_err(|e| format!("{}: {e}", config.display()))?;
    let c: LudusaviConfig =
        serde_yaml_ng::from_str(&text).map_err(|e| format!("{}: {e}", config.display()))?;
    let home = relicario_platform::home_dir().unwrap_or_default();
    let mut games = Vec::new();
    let mut skipped = Vec::new();
    for g in c.custom_games.into_iter().filter(|g| !g.ignore) {
        let mut roots = Vec::new();
        for (i, f) in g.files.iter().enumerate() {
            let p = f
                .replace("<home>", &home.to_string_lossy())
                .replace('\\', "/");
            if p.contains('<') {
                skipped.push(format!("{}: {f}", g.name));
                continue;
            }
            let (fixed, pattern) = relicario_catalog::resolve::split_static(&p);
            let name = if g.files.len() == 1 {
                "save".to_string()
            } else {
                format!("save{}", i + 1)
            };
            let (path, include) = match pattern {
                Some(pat) => (fixed, vec![pat]),
                None if fixed.is_file() => {
                    let file = fixed
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    (
                        fixed.parent().map(Path::to_path_buf).unwrap_or_default(),
                        vec![file],
                    )
                }
                None => (fixed, vec![]),
            };
            roots.push(RootConfig {
                name,
                path: path.to_string_lossy().into_owned(),
                include,
                exclude: vec![],
                mirrors: vec![],
            });
        }
        if !roots.is_empty() {
            games.push(GameConfig {
                id: relicario_catalog::slug::slug(&g.name),
                name: g.name,
                roots,
                executables: vec![],
            });
        }
    }
    Ok((games, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horarios() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2024-03-01T12:30:00.5Z"), Some(1709296200500));
        assert_eq!(parse_time("2024-03-01T09:30:00-03:00"), Some(1709296200000));
    }

    #[test]
    fn drives() {
        let mut d = BTreeMap::new();
        d.insert("drive-0".to_string(), "".to_string());
        d.insert("drive-C".to_string(), "C:".to_string());
        assert_eq!(
            backup_rel(&d, "/home/yuri/x.sav").as_deref(),
            Some("drive-0/home/yuri/x.sav")
        );
        assert_eq!(
            backup_rel(&d, "C:/Users/yuri/x.sav").as_deref(),
            Some("drive-C/Users/yuri/x.sav")
        );
    }
}
