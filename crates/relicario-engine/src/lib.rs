//! Motor de decisão do Relicário: compara históricos e decide entre igual, avanço, envio ou
//! divergência. Núcleo puro, sem I/O, para ser testado com históricos gerados aleatoriamente.
//!
//! Fase 0: só a relação entre duas cadeias lineares. A árvore completa, retenção e proteção
//! contra save corrompido chegam na fase 1.

use relicario_model::{Relation, SnapshotId};

/// Compara dois históricos lineares, do mais antigo para o mais novo.
pub fn relacao(local: &[SnapshotId], remoto: &[SnapshotId]) -> Relation {
    match (local.last(), remoto.last()) {
        (Some(a), Some(b)) if a == b => Relation::Igual,
        (None, None) => Relation::Igual,
        _ if local.starts_with(remoto) => Relation::Envio,
        _ if remoto.starts_with(local) => Relation::Avanco,
        _ => Relation::Divergiu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(ids: &[&str]) -> Vec<SnapshotId> {
        ids.iter().map(|s| SnapshotId::new(*s)).collect()
    }

    #[test]
    fn quatro_relacoes() {
        assert_eq!(relacao(&h(&["a", "b"]), &h(&["a", "b"])), Relation::Igual);
        assert_eq!(relacao(&h(&["a"]), &h(&["a", "b"])), Relation::Avanco);
        assert_eq!(relacao(&h(&["a", "b"]), &h(&["a"])), Relation::Envio);
        assert_eq!(
            relacao(&h(&["a", "b"]), &h(&["a", "c"])),
            Relation::Divergiu
        );
    }
}
