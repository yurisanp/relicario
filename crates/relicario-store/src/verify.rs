//! Conferência do repositório.

use crate::error::Result;
use crate::repo::Repository;
use relicario_model::{Hash, SnapshotId};
use relicario_transport::Kind;
use std::collections::HashSet;

/// Resultado de [`verify`].
#[derive(Debug, Clone, Default)]
pub struct VerifyReport {
    pub snapshots: usize,
    pub blobs_checked: usize,
    pub packs: usize,
    /// Problemas que significam dado perdido ou corrompido.
    pub errors: Vec<String>,
    /// Coisas inofensivas, como packs órfãos de uma escrita interrompida.
    pub warnings: Vec<String>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Confere metadados, árvores e a existência de todos os blocos. Com `read_data`, lê e confere
/// o hash de cada bloco usado.
pub fn verify(repo: &Repository, read_data: bool) -> Result<VerifyReport> {
    let mut r = VerifyReport::default();
    let forgotten: HashSet<SnapshotId> = repo
        .heads()?
        .into_iter()
        .flat_map(|h| h.forgotten.into_iter())
        .collect();
    let mut used: HashSet<Hash> = HashSet::new();
    for (id, s) in repo.snapshots()? {
        r.snapshots += 1;
        let s = match s {
            Ok(s) => s,
            Err(e) => {
                r.errors.push(format!("snapshot {}: {e}", id.short()));
                continue;
            }
        };
        if forgotten.contains(&id) {
            continue;
        }
        let tree = match repo.get_tree(&s.tree) {
            Ok(t) => t,
            Err(e) => {
                r.errors
                    .push(format!("árvore do snapshot {}: {e}", id.short()));
                continue;
            }
        };
        used.insert(s.tree);
        if tree.content_hash() != s.content {
            r.errors.push(format!(
                "snapshot {}: conteúdo não confere com a árvore",
                id.short()
            ));
        }
        for f in &tree.files {
            for c in &f.chunks {
                if used.insert(*c) && !repo.has_blob(c) {
                    r.errors.push(format!(
                        "snapshot {}: bloco {} ausente ({})",
                        id.short(),
                        c.short(),
                        f.path
                    ));
                }
            }
        }
    }
    let packs: HashSet<String> = repo.remote().list(Kind::Packs)?.into_iter().collect();
    r.packs = packs.len();
    let indexed: HashSet<String> = repo
        .index_entries()
        .values()
        .map(|l| l.pack.to_hex())
        .collect();
    for p in indexed.difference(&packs) {
        r.errors
            .push(format!("pack {} está no índice mas não existe", &p[..10]));
    }
    for p in packs.difference(&indexed) {
        r.warnings.push(format!(
            "pack {} órfão (escrita interrompida?); o prune remove",
            &p[..10.min(p.len())]
        ));
    }
    if read_data {
        let mut ids: Vec<&Hash> = used.iter().collect();
        // Ordem por pack e posição, para ler cada pack uma vez só.
        ids.sort_by_key(|id| repo.blob_location(id).map(|l| (l.pack, l.offset)));
        for id in ids {
            if !repo.has_blob(id) {
                continue;
            }
            match repo.get_blob(id) {
                Ok(_) => r.blobs_checked += 1,
                Err(e) => r.errors.push(format!("bloco {}: {e}", id.short())),
            }
        }
    }
    Ok(r)
}
