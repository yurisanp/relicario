//! `relicario-restore`: tira saves de um repositório do Relicário sem o daemon, sem a
//! configuração e sem mudar nada no repositório.
//!
//! Serve para a pior hora: a máquina morreu, o Relicário não abre, ou você só tem a cópia da
//! nuvem (uma pasta do Syncthing, do Dropbox, ou o que baixou do Drive/S3). Abre a pasta só para
//! leitura, lista jogos e versões, e extrai uma versão para uma pasta qualquer.

use clap::{Parser, Subcommand};
use relicario_model::{GameId, Snapshot, SnapshotId};
use relicario_store::crypto::Encrypted;
use relicario_store::files;
use relicario_store::time::format_utc;
use relicario_store::{Repository, verify};
use relicario_transport::{Kind, LocalFolder, Remote};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "relicario-restore",
    version,
    about = "Recupera saves de um repositório do Relicário, só lendo"
)]
struct Cli {
    /// Pasta do repositório (o local, ou uma cópia da nuvem)
    repo: PathBuf,
    /// Senha, se a cópia é cifrada (ou use RELICARIO_SENHA; sem nada, pergunta)
    #[arg(long, global = true)]
    senha: Option<String>,
    /// Chave de recuperação, no lugar da senha
    #[arg(long, global = true, value_name = "CHAVE")]
    recuperacao: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Jogos no repositório
    Jogos,
    /// Versões de um jogo, da mais nova para a mais velha
    Versoes { jogo: String },
    /// Escreve uma versão numa pasta: `<destino>/<raiz>/<arquivo>`
    Extrair {
        /// Id (ou começo do id) de uma versão, ou o nome de um jogo para a mais nova dele
        versao: String,
        destino: PathBuf,
    },
    /// Confere se todos os dados do repositório estão inteiros
    Conferir,
}

/// Recusa qualquer escrita: o repositório fica exatamente como estava.
#[derive(Debug)]
struct ReadOnly(Arc<dyn Remote>);

impl Remote for ReadOnly {
    fn put(&self, _: Kind, _: &str, _: &[u8]) -> relicario_transport::Result<()> {
        Err(relicario_transport::Error::Other(
            "aberto só para leitura".into(),
        ))
    }
    fn get(&self, kind: Kind, name: &str) -> relicario_transport::Result<Option<Vec<u8>>> {
        self.0.get(kind, name)
    }
    fn list(&self, kind: Kind) -> relicario_transport::Result<Vec<String>> {
        self.0.list(kind)
    }
    fn delete(&self, _: Kind, _: &str) -> relicario_transport::Result<()> {
        Err(relicario_transport::Error::Other(
            "aberto só para leitura".into(),
        ))
    }
    fn describe(&self) -> String {
        format!("{} (só leitura)", self.0.describe())
    }
}

type R<T> = Result<T, String>;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("relicario-restore: {e}");
            ExitCode::FAILURE
        }
    }
}

fn open(cli: &Cli) -> R<Repository> {
    if !cli.repo.is_dir() {
        return Err(format!("{} não é uma pasta", cli.repo.display()));
    }
    let folder: Arc<dyn Remote> = Arc::new(LocalFolder::new(&cli.repo));
    let encrypted =
        relicario_store::crypto::is_encrypted(folder.as_ref()).map_err(|e| e.to_string())?;
    let remote: Arc<dyn Remote> = if encrypted {
        let enc = if let Some(k) = &cli.recuperacao {
            Encrypted::open_with_recovery(folder, k)
        } else {
            let pw = match cli
                .senha
                .clone()
                .or_else(|| std::env::var("RELICARIO_SENHA").ok())
            {
                Some(p) => p,
                None => rpassword::prompt_password("senha do repositório: ")
                    .map_err(|e| e.to_string())?,
            };
            Encrypted::open_with_password(folder, &pw)
        }
        .map_err(|e| e.to_string())?;
        Arc::new(ReadOnly(Arc::new(enc)))
    } else {
        Arc::new(ReadOnly(folder))
    };
    Repository::open(remote).map_err(|e| format!("{}: {e}", cli.repo.display()))
}

struct Index {
    snaps: Vec<(SnapshotId, Snapshot)>,
    names: BTreeMap<String, String>,
    pins: BTreeMap<SnapshotId, String>,
    heads: BTreeMap<SnapshotId, Vec<String>>,
}

fn index(repo: &Repository) -> R<Index> {
    let mut snaps = repo.valid_snapshots().map_err(|e| e.to_string())?;
    snaps.sort_by_key(|(id, s)| (std::cmp::Reverse(s.created_at), *id));
    let mut names = BTreeMap::new();
    let mut pins = BTreeMap::new();
    let mut heads: BTreeMap<SnapshotId, Vec<String>> = BTreeMap::new();
    for h in repo.heads().map_err(|e| e.to_string())? {
        let name = if h.machine_name.is_empty() {
            h.machine
                .as_ref()
                .map(|m| m.to_string())
                .unwrap_or_default()
        } else {
            h.machine_name.clone()
        };
        if let Some(m) = &h.machine {
            names.insert(m.to_string(), name.clone());
        }
        pins.extend(h.pins.clone());
        for x in h.heads.values() {
            heads.entry(x.snapshot).or_default().push(name.clone());
        }
    }
    Ok(Index {
        snaps,
        names,
        pins,
        heads,
    })
}

fn run(cli: Cli) -> R<()> {
    let repo = open(&cli)?;
    let ix = index(&repo)?;
    match &cli.cmd {
        Cmd::Jogos => {
            let mut games: BTreeMap<&str, (usize, i64)> = BTreeMap::new();
            for (_, s) in &ix.snaps {
                let e = games.entry(s.game.as_str()).or_default();
                e.0 += 1;
                e.1 = e.1.max(s.created_at);
            }
            if games.is_empty() {
                println!("nenhum jogo neste repositório");
            }
            for (g, (n, last)) in games {
                println!("{g}: {n} versão(ões), a última em {} UTC", format_utc(last));
            }
        }
        Cmd::Versoes { jogo } => {
            let game = GameId::new(jogo);
            let list: Vec<_> = ix.snaps.iter().filter(|(_, s)| s.game == game).collect();
            if list.is_empty() {
                return Err(format!("nenhuma versão de {jogo} (veja `jogos`)"));
            }
            for (id, s) in list {
                let mut extra = Vec::new();
                if let Some(p) = ix.pins.get(id) {
                    extra.push(format!("fixada: {p}"));
                }
                if let Some(m) = ix.heads.get(id) {
                    extra.push(format!("atual em {}", m.join(", ")));
                }
                let available = repo.get_tree(&s.tree).is_ok();
                if !available {
                    extra.push("só os metadados (dados apagados pela retenção)".into());
                }
                println!(
                    "{}  {} UTC  {}  {:?}  {} arquivo(s), {} bytes{}",
                    id.short(),
                    format_utc(s.created_at),
                    ix.names
                        .get(s.machine.as_str())
                        .map(String::as_str)
                        .unwrap_or(s.machine.as_str()),
                    s.reason,
                    s.files,
                    s.size,
                    if extra.is_empty() {
                        String::new()
                    } else {
                        format!("  [{}]", extra.join("; "))
                    }
                );
            }
        }
        Cmd::Extrair { versao, destino } => {
            let (id, s) = pick(&repo, &ix, versao)?;
            let n = extract(&repo, s, destino)?;
            println!(
                "{} arquivo(s) da versão {} de {} ({} UTC) em {}",
                n,
                id.short(),
                s.game,
                format_utc(s.created_at),
                destino.display()
            );
        }
        Cmd::Conferir => {
            let r = verify::verify(&repo, true).map_err(|e| e.to_string())?;
            for w in &r.warnings {
                println!("aviso: {w}");
            }
            for e in &r.errors {
                println!("ERRO: {e}");
            }
            println!(
                "{} versões, {} blocos conferidos em {} packs: {}",
                r.snapshots,
                r.blobs_checked,
                r.packs,
                if r.ok() {
                    "tudo inteiro"
                } else {
                    "há dados perdidos"
                }
            );
            if !r.ok() {
                return Err("o repositório tem dados perdidos".into());
            }
        }
    }
    Ok(())
}

/// Uma versão pelo id (ou começo dele), ou a mais nova de um jogo, entre as que têm dados.
fn pick<'a>(repo: &Repository, ix: &'a Index, what: &str) -> R<(SnapshotId, &'a Snapshot)> {
    let has_data = |s: &Snapshot| repo.get_tree(&s.tree).is_ok();
    let by_id: Vec<_> = ix
        .snaps
        .iter()
        .filter(|(id, _)| what.len() >= 4 && id.to_hex().starts_with(&what.to_lowercase()))
        .collect();
    match by_id.as_slice() {
        [(id, s)] => return Ok((*id, s)),
        [] => {}
        _ => {
            return Err(format!(
                "{what} combina com mais de uma versão; use mais caracteres"
            ));
        }
    }
    let game = GameId::new(what);
    ix.snaps
        .iter()
        .find(|(_, s)| s.game == game && has_data(s))
        .map(|(id, s)| (*id, s))
        .ok_or_else(|| format!("nenhuma versão ou jogo chamado {what} (veja `jogos`)"))
}

fn extract(repo: &Repository, s: &Snapshot, dest: &Path) -> R<usize> {
    let tree = repo
        .get_tree(&s.tree)
        .map_err(|e| format!("os dados desta versão não estão no repositório ({e})"))?;
    for f in &tree.files {
        let mut p = dest.join(safe(&f.root)?);
        for part in f.path.split('/') {
            p.push(safe(part)?);
        }
        let data = files::read_entry(repo, f).map_err(|e| format!("{}: {e}", f.path))?;
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&p, data).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(tree.files.len())
}

fn safe(part: &str) -> R<&str> {
    if part.is_empty() || part == "." || part == ".." || part.contains(['/', '\\', ':']) {
        return Err(format!("caminho inseguro no repositório: {part:?}"));
    }
    Ok(part)
}
