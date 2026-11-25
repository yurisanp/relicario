//! Operações de alto nível sobre um jogo: snapshot, plano, restauração e retenção. São usadas
//! pela CLI e pelo daemon, e garantem as regras de segurança do roadmap:
//!
//! - nunca restaurar sem antes guardar o estado atual do disco;
//! - snapshot suspeito (guarda contra save corrompido) é guardado, mas não vira head sem
//!   confirmação.

use crate::error::{Error, Result};
use crate::files::{self, Source};
use crate::repo::Repository;
use relicario_engine::guard::{self, GuardPolicy};
use relicario_engine::retention::{self, Candidate, Policy};
use relicario_engine::{Graph, Node, Plan, PlanInput};
use relicario_model::{
    GameId, Head, HeadsFile, MachineId, Snapshot, SnapshotId, SnapshotReason, Suspicion, Tree,
};
use std::collections::{BTreeMap, HashSet};

/// Um jogo com as raízes de save nesta máquina.
#[derive(Debug, Clone)]
pub struct GameSpec {
    pub id: GameId,
    pub sources: Vec<Source>,
}

/// Contexto desta máquina.
#[derive(Debug)]
pub struct Local<'a> {
    pub repo: &'a mut Repository,
    pub machine: MachineId,
    pub machine_name: String,
}

impl Local<'_> {
    /// Arquivo de heads desta máquina (vazio se ainda não existe).
    pub fn heads(&self) -> Result<HeadsFile> {
        Ok(self
            .repo
            .heads_of(&self.machine)?
            .unwrap_or_else(|| HeadsFile {
                machine: Some(self.machine.clone()),
                machine_name: self.machine_name.clone(),
                ..HeadsFile::default()
            }))
    }

    fn save_heads(&mut self, mut h: HeadsFile) -> Result<()> {
        h.machine = Some(self.machine.clone());
        h.machine_name = self.machine_name.clone();
        // Sempre cresce, mesmo com o relógio voltando: a replicação fica com o maior.
        h.updated_at = crate::time::now().max(h.updated_at + 1);
        let m = self.machine.clone();
        self.repo.put_heads(&m, &h)
    }

    pub fn head_of(&self, game: &GameId) -> Result<Option<SnapshotId>> {
        Ok(self.heads()?.heads.get(game).map(|h| h.snapshot))
    }

    fn set_head(&mut self, game: &GameId, id: SnapshotId) -> Result<()> {
        let mut h = self.heads()?;
        h.heads.insert(
            game.clone(),
            Head {
                snapshot: id,
                updated_at: crate::time::now(),
            },
        );
        self.save_heads(h)
    }
}

/// Resultado de [`take_snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotOutcome {
    /// O disco está igual ao head: nada foi gravado.
    Unchanged(Option<SnapshotId>),
    /// Snapshot novo, que virou head.
    Created(SnapshotId),
    /// Snapshot novo, guardado mas não promovido a head por parecer corrompido.
    Suspect(SnapshotId, Vec<Suspicion>),
}

/// Tira um snapshot das raízes do jogo. Com `force`, um snapshot suspeito vira head mesmo assim.
pub fn take_snapshot(
    l: &mut Local<'_>,
    game: &GameSpec,
    reason: SnapshotReason,
    force: bool,
) -> Result<SnapshotOutcome> {
    let _lock = l.repo.lock(&l.machine, crate::lock::LockMode::Shared)?;
    let head = l.head_of(&game.id)?;
    let head_snap = head.map(|h| l.repo.get_snapshot(&h)).transpose()?;
    // Comparação barata antes de gravar blocos.
    let disk = files::scan(&game.sources)?;
    if let Some(s) = &head_snap
        && s.content == disk.content_hash()
    {
        return Ok(SnapshotOutcome::Unchanged(head));
    }
    if head.is_none() && disk.files.is_empty() {
        return Ok(SnapshotOutcome::Unchanged(None));
    }
    let tree = files::store_files(l.repo, &game.sources)?;
    let prev_tree = head_snap
        .as_ref()
        .map(|s| l.repo.get_tree(&s.tree))
        .transpose()?;
    let suspicions = guard::check(prev_tree.as_ref(), &tree, GuardPolicy::default());
    let id = write_snapshot(
        l,
        game,
        &tree,
        head.into_iter().collect(),
        reason,
        suspicions.clone(),
    )?;
    if suspicions.is_empty() || force {
        l.set_head(&game.id, id)?;
        Ok(SnapshotOutcome::Created(id))
    } else {
        Ok(SnapshotOutcome::Suspect(id, suspicions))
    }
}

fn write_snapshot(
    l: &mut Local<'_>,
    game: &GameSpec,
    tree: &Tree,
    parents: Vec<SnapshotId>,
    reason: SnapshotReason,
    suspicions: Vec<Suspicion>,
) -> Result<SnapshotId> {
    let tree_id = l.repo.put_tree(tree)?;
    let s = Snapshot {
        game: game.id.clone(),
        machine: l.machine.clone(),
        parents,
        tree: tree_id,
        content: tree.content_hash(),
        created_at: crate::time::now(),
        mtime_max: tree.files.iter().map(|f| f.mtime).max().unwrap_or(0),
        size: tree.total_size(),
        files: tree.files.len() as u64,
        reason,
        suspicions,
        meta: BTreeMap::new(),
    };
    l.repo.put_snapshot(&s)
}

/// Snapshots de um jogo, em ordem de criação.
pub fn game_snapshots(repo: &Repository, game: &GameId) -> Result<Vec<(SnapshotId, Snapshot)>> {
    let mut v: Vec<_> = repo
        .valid_snapshots()?
        .into_iter()
        .filter(|(_, s)| &s.game == game)
        .collect();
    v.sort_by_key(|(id, s)| (s.created_at, *id));
    Ok(v)
}

/// Grafo do jogo.
pub fn graph(repo: &Repository, game: &GameId) -> Result<Graph> {
    let mut g = Graph::new();
    for (id, s) in game_snapshots(repo, game)? {
        g.insert(id, Node::from(&s));
    }
    Ok(g)
}

/// Heads das outras máquinas para um jogo.
pub fn remote_heads(l: &Local<'_>, game: &GameId) -> Result<Vec<(MachineId, SnapshotId)>> {
    let mut out = Vec::new();
    for h in l.repo.heads()? {
        let Some(m) = h.machine.clone() else { continue };
        if m == l.machine {
            continue;
        }
        if let Some(x) = h.heads.get(game) {
            out.push((m, x.snapshot));
        }
    }
    Ok(out)
}

/// Calcula o plano do jogo.
pub fn plan(l: &Local<'_>, game: &GameSpec) -> Result<(Plan, Tree)> {
    let disk = files::scan(&game.sources)?;
    let g = graph(l.repo, &game.id)?;
    let input = PlanInput {
        graph: &g,
        disk: (!disk.files.is_empty()).then(|| disk.content_hash()),
        local_head: l.head_of(&game.id)?,
        remotes: remote_heads(l, &game.id)?,
    };
    Ok((relicario_engine::plan(&input), disk))
}

/// Resultado de [`restore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    /// Snapshot de segurança do estado anterior, quando o disco tinha algo não guardado.
    pub safety: Option<SnapshotId>,
    pub report: files::RestoreReport,
}

/// Restaura `target` no disco. Antes, guarda o estado atual se ele ainda não estiver em nenhum
/// snapshot do jogo. Depois, confere que o disco ficou idêntico ao snapshot.
pub fn restore(l: &mut Local<'_>, game: &GameSpec, target: SnapshotId) -> Result<RestoreOutcome> {
    let snap = l.repo.get_snapshot(&target)?;
    if snap.game != game.id {
        return Err(Error::Corrupt(format!(
            "snapshot {} é do jogo {}, não de {}",
            target.short(),
            snap.game,
            game.id
        )));
    }
    let tree = l.repo.get_tree(&snap.tree)?;
    let _lock = l.repo.lock(&l.machine, crate::lock::LockMode::Shared)?;
    let disk = files::scan(&game.sources)?;
    let mut safety = None;
    if !disk.files.is_empty() {
        let c = disk.content_hash();
        // Só conta como guardado se os dados ainda existem (não foi esquecido pela retenção).
        let forgotten: HashSet<SnapshotId> = l
            .repo
            .heads()?
            .into_iter()
            .flat_map(|h| h.forgotten)
            .collect();
        let known = game_snapshots(l.repo, &game.id)?
            .iter()
            .any(|(id, s)| s.content == c && !forgotten.contains(id));
        if !known {
            let head = l.head_of(&game.id)?;
            let stored = files::store_files(l.repo, &game.sources)?;
            let id = write_snapshot(
                l,
                game,
                &stored,
                head.into_iter().collect(),
                SnapshotReason::BeforeRestore,
                vec![],
            )?;
            safety = Some(id);
        }
    }
    let report = files::restore(l.repo, &tree, &game.sources)?;
    let after = files::scan(&game.sources)?;
    if after.content_hash() != tree.content_hash() {
        return Err(Error::Corrupt(
            "o disco não ficou igual ao snapshot depois de restaurar".into(),
        ));
    }
    l.set_head(&game.id, target)?;
    Ok(RestoreOutcome { safety, report })
}

/// Move o head sem tocar no disco (o conteúdo já é o mesmo).
pub fn adopt(l: &mut Local<'_>, game: &GameId, target: SnapshotId) -> Result<()> {
    l.set_head(game, target)
}

/// Fixa um snapshot com um nome. Fixados nunca são liberados pela retenção.
pub fn pin(l: &mut Local<'_>, id: SnapshotId, name: String) -> Result<()> {
    l.repo.get_snapshot(&id)?;
    let mut h = l.heads()?;
    h.pins.insert(id, name);
    h.forgotten.remove(&id);
    l.save_heads(h)
}

pub fn unpin(l: &mut Local<'_>, id: SnapshotId) -> Result<bool> {
    let mut h = l.heads()?;
    let had = h.pins.remove(&id).is_some();
    l.save_heads(h)?;
    Ok(had)
}

/// Marca como esquecidos os snapshots desta máquina que a política não guarda mais. Os dados só
/// são apagados de fato pelo [`crate::prune::prune`].
pub fn apply_retention(l: &mut Local<'_>, policy: Policy, now: i64) -> Result<Vec<SnapshotId>> {
    let all_heads = l.repo.heads()?;
    let mut protected: HashSet<SnapshotId> = HashSet::new();
    for h in &all_heads {
        protected.extend(h.heads.values().map(|x| x.snapshot));
        protected.extend(h.pins.keys().copied());
    }
    let mut mine = l.heads()?;
    let snaps = l.repo.valid_snapshots()?;
    let mut by_game: BTreeMap<GameId, Vec<Candidate>> = BTreeMap::new();
    for (id, s) in &snaps {
        if s.machine == l.machine && !mine.forgotten.contains(id) {
            by_game.entry(s.game.clone()).or_default().push(Candidate {
                id: *id,
                created_at: s.created_at,
            });
        }
    }
    let mut out = Vec::new();
    for cands in by_game.values() {
        out.extend(retention::select_forget(cands, &protected, now, policy));
    }
    if !out.is_empty() {
        mine.forgotten.extend(out.iter().copied());
        l.save_heads(mine)?;
    }
    Ok(out)
}

/// Marca o jogo como aberto (ou fechado) nesta máquina.
pub fn set_running(l: &mut Local<'_>, game: &GameId, since: Option<i64>) -> Result<()> {
    let mut h = l.heads()?;
    match since {
        Some(t) => {
            h.running.insert(game.clone(), t);
        }
        None => {
            h.running.remove(game);
        }
    }
    l.save_heads(h)
}

/// Soma tempo de jogo.
pub fn add_playtime(l: &mut Local<'_>, game: &GameId, ms: u64) -> Result<()> {
    let mut h = l.heads()?;
    *h.playtime.entry(game.clone()).or_default() += ms;
    l.save_heads(h)
}

/// Publica os endereços desta máquina (o "cartão de visita" para a sincronização direta).
pub fn set_addresses(l: &mut Local<'_>, addrs: Vec<String>) -> Result<()> {
    let mut h = l.heads()?;
    if h.addresses == addrs {
        return Ok(());
    }
    h.addresses = addrs;
    l.save_heads(h)
}

/// Associa um executável a um jogo.
pub fn associate(l: &mut Local<'_>, game: &GameId, exe_name: &str) -> Result<()> {
    let mut h = l.heads()?;
    let v = h.associations.entry(game.clone()).or_default();
    if !v.iter().any(|x| x.eq_ignore_ascii_case(exe_name)) {
        v.push(exe_name.to_string());
    }
    l.save_heads(h)
}

/// Outras máquinas com o jogo aberto agora: (nome da máquina, desde quando). Marcas com mais de
/// 12 horas são ignoradas (máquina que desligou sem avisar).
pub fn open_elsewhere(l: &Local<'_>, game: &GameId, now: i64) -> Result<Vec<(String, i64)>> {
    let mut out = Vec::new();
    for h in l.repo.heads()? {
        if h.machine.as_ref() == Some(&l.machine) {
            continue;
        }
        if let Some(t) = h.running.get(game)
            && now - t < 12 * 3_600_000
        {
            let name = if h.machine_name.is_empty() {
                h.machine.map(|m| m.to_string()).unwrap_or_default()
            } else {
                h.machine_name
            };
            out.push((name, *t));
        }
    }
    Ok(out)
}

/// Executáveis associados a cada jogo, juntando todas as máquinas.
pub fn all_associations(repo: &Repository) -> Result<BTreeMap<GameId, Vec<String>>> {
    let mut out: BTreeMap<GameId, Vec<String>> = BTreeMap::new();
    for h in repo.heads()? {
        for (g, exes) in h.associations {
            let v = out.entry(g).or_default();
            for e in exes {
                if !v.iter().any(|x| x.eq_ignore_ascii_case(&e)) {
                    v.push(e);
                }
            }
        }
    }
    Ok(out)
}

/// Tempo de jogo por máquina (nome da máquina → ms).
pub fn playtime_by_machine(repo: &Repository, game: &GameId) -> Result<Vec<(String, u64)>> {
    let mut out = Vec::new();
    for h in repo.heads()? {
        if let Some(ms) = h.playtime.get(game) {
            let name = if h.machine_name.is_empty() {
                h.machine.map(|m| m.to_string()).unwrap_or_default()
            } else {
                h.machine_name
            };
            out.push((name, *ms));
        }
    }
    Ok(out)
}
