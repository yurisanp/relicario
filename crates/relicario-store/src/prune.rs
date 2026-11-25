//! Coleta de lixo: libera os dados de snapshots esquecidos e remove packs órfãos.

use crate::error::Result;
use crate::lock::LockMode;
use crate::pack::{IndexFile, IndexPack, PackBuilder};
use crate::repo::Repository;
use relicario_model::{Hash, MachineId, SnapshotId};
use relicario_transport::Kind;
use std::collections::{HashMap, HashSet};

/// O que o prune fez.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    pub packs_removed: usize,
    pub packs_rewritten: usize,
    pub blobs_removed: usize,
    pub bytes_freed: u64,
}

/// Remove blocos que nenhum snapshot ativo usa. Snapshots listados em `forgotten` de qualquer
/// arquivo de heads não contam como ativos, mas os metadados deles continuam no repositório.
///
/// Pega a trava exclusiva: nenhuma outra máquina pode estar escrevendo.
pub fn prune(repo: &mut Repository, machine: &MachineId) -> Result<PruneReport> {
    let _lock = repo.lock(machine, LockMode::Exclusive)?;
    repo.flush()?;
    repo.rebuild_index_cache()?;
    let mut report = PruneReport::default();

    let heads = repo.heads()?;
    let forgotten: HashSet<SnapshotId> = heads
        .iter()
        .flat_map(|h| h.forgotten.iter().copied())
        .collect();
    let protected: HashSet<SnapshotId> = heads
        .iter()
        .flat_map(|h| h.heads.values().map(|x| x.snapshot))
        .collect();

    // Blocos em uso. Na dúvida (snapshot ilegível), não apaga nada.
    let mut used: HashSet<Hash> = HashSet::new();
    for (id, s) in repo.snapshots()? {
        let s = s?;
        if forgotten.contains(&id) && !protected.contains(&id) {
            continue;
        }
        let tree = repo.get_tree(&s.tree)?;
        used.insert(s.tree);
        for f in tree.files {
            used.extend(f.chunks);
        }
    }

    // Agrupar o índice por pack.
    let mut by_pack: HashMap<Hash, Vec<(Hash, crate::pack::BlobLoc)>> = HashMap::new();
    for (id, loc) in repo.index_entries() {
        by_pack.entry(loc.pack).or_default().push((*id, *loc));
    }
    let existing: HashSet<String> = repo.remote().list(Kind::Packs)?.into_iter().collect();

    let mut keep_packs: Vec<IndexPack> = Vec::new();
    let mut drop_packs: Vec<Hash> = Vec::new();
    let mut rewrite = PackBuilder::default();
    let mut new_packs: Vec<IndexPack> = Vec::new();
    for (pack, mut blobs) in by_pack {
        blobs.sort_by_key(|(_, l)| l.offset);
        let (vivos, mortos): (Vec<_>, Vec<_>) = blobs.iter().partition(|(id, _)| used.contains(id));
        if mortos.is_empty() {
            keep_packs.push(IndexPack {
                id: pack,
                blobs: vivos
                    .iter()
                    .map(|(id, l)| crate::pack::IndexBlob {
                        id: *id,
                        offset: l.offset,
                        length: l.length,
                        raw_length: l.raw_length,
                    })
                    .collect(),
            });
            continue;
        }
        report.blobs_removed += mortos.len();
        report.bytes_freed += mortos.iter().map(|(_, l)| l.length).sum::<u64>();
        if !vivos.is_empty() {
            let data = repo.load_pack(&pack)?;
            for (id, l) in &vivos {
                let stored = &data[l.offset as usize..(l.offset + l.length) as usize];
                rewrite.push(*id, stored, l.raw_length);
                if rewrite.len() >= repo.config().pack_size {
                    new_packs.push(write_pack(repo, std::mem::take(&mut rewrite))?);
                }
            }
            report.packs_rewritten += 1;
        } else {
            report.packs_removed += 1;
        }
        drop_packs.push(pack);
    }
    if !rewrite.is_empty() {
        new_packs.push(write_pack(repo, rewrite)?);
    }

    // Packs órfãos (sem índice) de escritas interrompidas.
    let indexed: HashSet<String> = keep_packs
        .iter()
        .chain(new_packs.iter())
        .map(|p| p.id.to_hex())
        .chain(drop_packs.iter().map(|h| h.to_hex()))
        .collect();
    let orphans: Vec<String> = existing.difference(&indexed).cloned().collect();

    if drop_packs.is_empty() && orphans.is_empty() {
        return Ok(report);
    }

    // Ordem segura: novos packs e o índice consolidado primeiro; depois apagar índices antigos;
    // por último os packs antigos.
    let old_index: Vec<String> = repo.index_file_names().to_vec();
    let mut all = keep_packs;
    all.extend(new_packs);
    let new_name = repo.write_index(&IndexFile {
        version: 1,
        packs: all,
    })?;
    for n in old_index {
        if n != new_name {
            repo.delete(Kind::Index, &n)?;
        }
    }
    for p in &drop_packs {
        repo.delete(Kind::Packs, &p.to_hex())?;
    }
    for p in &orphans {
        repo.delete(Kind::Packs, p)?;
        report.packs_removed += 1;
    }
    repo.rebuild_index_cache()?;
    Ok(report)
}

fn write_pack(repo: &mut Repository, pb: PackBuilder) -> Result<IndexPack> {
    let id = Hash::of(&pb.data);
    repo.remote().put(Kind::Packs, &id.to_hex(), &pb.data)?;
    Ok(IndexPack {
        id,
        blobs: pb.blobs,
    })
}
