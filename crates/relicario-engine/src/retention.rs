//! Retenção: quais snapshots desta máquina podem ter os dados liberados.
//!
//! Política padrão: tudo das últimas 24 horas, o mais recente de cada dia nos últimos 30 dias,
//! o mais recente de cada mês (30 dias) no último ano. Heads de qualquer máquina, snapshots
//! fixados e o mais recente de cada jogo nunca são liberados.

use relicario_model::{Millis, SnapshotId};
use std::collections::{BTreeMap, BTreeSet, HashSet};

const HORA: Millis = 3_600_000;
const DIA: Millis = 24 * HORA;

/// Janelas de retenção.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub keep_all: Millis,
    pub daily: Millis,
    pub monthly: Millis,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            keep_all: DIA,
            daily: 30 * DIA,
            monthly: 365 * DIA,
        }
    }
}

/// Um snapshot candidato.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub id: SnapshotId,
    pub created_at: Millis,
}

/// Devolve os snapshots que podem ser esquecidos. `protected` são heads e fixados.
pub fn select_forget(
    candidates: &[Candidate],
    protected: &HashSet<SnapshotId>,
    now: Millis,
    policy: Policy,
) -> BTreeSet<SnapshotId> {
    let mut keep: HashSet<SnapshotId> = protected.clone();
    if let Some(newest) = candidates.iter().max_by_key(|c| (c.created_at, c.id)) {
        keep.insert(newest.id);
    }
    // Mais recente por balde (dia ou mês).
    let mut buckets: BTreeMap<(u8, Millis), Candidate> = BTreeMap::new();
    for c in candidates {
        let age = now - c.created_at;
        if age <= policy.keep_all {
            keep.insert(c.id);
            continue;
        }
        let key = if age <= policy.daily {
            (0, c.created_at.div_euclid(DIA))
        } else if age <= policy.monthly {
            (1, c.created_at.div_euclid(30 * DIA))
        } else {
            continue;
        };
        let e = buckets.entry(key).or_insert(*c);
        if (c.created_at, c.id) > (e.created_at, e.id) {
            *e = *c;
        }
    }
    keep.extend(buckets.values().map(|c| c.id));
    candidates
        .iter()
        .filter(|c| !keep.contains(&c.id))
        .map(|c| c.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use relicario_model::Hash;

    fn c(n: u8, created_at: Millis) -> Candidate {
        Candidate {
            id: Hash::from_bytes([n; 32]),
            created_at,
        }
    }

    #[test]
    fn janelas() {
        let now = 400 * DIA;
        let cs = vec![
            c(1, now - HORA),           // 24 h: fica
            c(2, now - 2 * HORA),       // 24 h: fica
            c(3, now - 3 * DIA),        // dia X: perde para o 4
            c(4, now - 3 * DIA + HORA), // dia X: fica
            c(5, now - 100 * DIA),      // mês: fica (único)
            c(6, now - 380 * DIA),      // mais de 1 ano: sai
            c(7, now - 390 * DIA),      // mais de 1 ano, mas fixado
        ];
        let protected = HashSet::from([Hash::from_bytes([7; 32])]);
        let f = select_forget(&cs, &protected, now, Policy::default());
        assert_eq!(
            f,
            BTreeSet::from([Hash::from_bytes([3; 32]), Hash::from_bytes([6; 32])])
        );
    }

    #[test]
    fn nunca_esquece_o_mais_recente() {
        let now = 1000 * DIA;
        let cs = vec![c(1, 0)];
        assert!(select_forget(&cs, &HashSet::new(), now, Policy::default()).is_empty());
    }
}
