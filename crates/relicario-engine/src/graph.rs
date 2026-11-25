//! A árvore de snapshots.

use relicario_model::{Hash, MachineId, Millis, Relation, Snapshot, SnapshotId};
use std::collections::{HashMap, HashSet, VecDeque};

/// O que o motor precisa saber de um snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub parents: Vec<SnapshotId>,
    pub content: Hash,
    pub machine: MachineId,
    pub created_at: Millis,
}

impl From<&Snapshot> for Node {
    fn from(s: &Snapshot) -> Self {
        Node {
            parents: s.parents.clone(),
            content: s.content,
            machine: s.machine.clone(),
            created_at: s.created_at,
        }
    }
}

/// Grafo de snapshots conhecidos de um jogo. Pais ausentes são permitidos: a história pode
/// estar incompleta enquanto blocos ainda chegam.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    nodes: HashMap<SnapshotId, Node>,
}

/// Resultado de uma busca de ancestralidade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ancestry {
    Sim,
    Nao,
    /// Não encontrado, mas a busca passou por um pai desconhecido.
    Incompleto,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, id: SnapshotId, node: Node) {
        self.nodes.insert(id, node);
    }

    pub fn get(&self, id: &SnapshotId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn contains(&self, id: &SnapshotId) -> bool {
        self.nodes.contains_key(id)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = &SnapshotId> {
        self.nodes.keys()
    }

    /// `a` é ancestral de `b` (ou o próprio `b`)?
    pub fn is_ancestor(&self, a: &SnapshotId, b: &SnapshotId) -> Ancestry {
        if a == b {
            return Ancestry::Sim;
        }
        let mut incompleto = false;
        let mut vistos = HashSet::new();
        let mut fila = VecDeque::from([*b]);
        while let Some(id) = fila.pop_front() {
            if !vistos.insert(id) {
                continue;
            }
            if &id == a {
                return Ancestry::Sim;
            }
            match self.nodes.get(&id) {
                Some(n) => fila.extend(n.parents.iter().copied()),
                None => incompleto = true,
            }
        }
        if incompleto {
            Ancestry::Incompleto
        } else {
            Ancestry::Nao
        }
    }

    /// Conteúdo de um snapshot, se conhecido.
    pub fn content(&self, id: &SnapshotId) -> Option<Hash> {
        self.nodes.get(id).map(|n| n.content)
    }

    /// Relação entre o head local e o de outra máquina. Pai desconhecido conta como
    /// divergência até a história completar, nunca como avanço.
    pub fn relation(&self, local: Option<&SnapshotId>, remote: Option<&SnapshotId>) -> Relation {
        match (local, remote) {
            (None, None) => Relation::Igual,
            (None, Some(_)) => Relation::Avanco,
            (Some(_), None) => Relation::Envio,
            (Some(l), Some(r)) => {
                if l == r {
                    return Relation::Igual;
                }
                if let (Some(cl), Some(cr)) = (self.content(l), self.content(r))
                    && cl == cr
                {
                    return Relation::Igual;
                }
                match (self.is_ancestor(l, r), self.is_ancestor(r, l)) {
                    (Ancestry::Sim, _) => Relation::Avanco,
                    (_, Ancestry::Sim) => Relation::Envio,
                    _ => Relation::Divergiu,
                }
            }
        }
    }

    /// Ancestrais de `id` (incluindo ele), até onde a história é conhecida.
    pub fn ancestors(&self, id: &SnapshotId) -> HashSet<SnapshotId> {
        let mut vistos = HashSet::new();
        let mut fila = VecDeque::from([*id]);
        while let Some(x) = fila.pop_front() {
            if vistos.insert(x)
                && let Some(n) = self.nodes.get(&x)
            {
                fila.extend(n.parents.iter().copied());
            }
        }
        vistos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn id(n: u8) -> SnapshotId {
        Hash::from_bytes([n; 32])
    }

    fn node(parents: &[u8], content: u8) -> Node {
        Node {
            parents: parents.iter().map(|p| id(*p)).collect(),
            content: Hash::from_bytes([content + 100; 32]),
            machine: MachineId::new("m"),
            created_at: 0,
        }
    }

    #[test]
    fn quatro_relacoes_e_historia_incompleta() {
        let mut g = Graph::new();
        g.insert(id(1), node(&[], 1));
        g.insert(id(2), node(&[1], 2));
        g.insert(id(3), node(&[2], 3));
        g.insert(id(4), node(&[2], 4));
        g.insert(id(5), node(&[9], 5)); // pai 9 desconhecido
        g.insert(id(6), node(&[1], 2)); // mesmo conteúdo de 2
        assert_eq!(g.relation(Some(&id(3)), Some(&id(3))), Relation::Igual);
        assert_eq!(g.relation(Some(&id(1)), Some(&id(3))), Relation::Avanco);
        assert_eq!(g.relation(Some(&id(3)), Some(&id(1))), Relation::Envio);
        assert_eq!(g.relation(Some(&id(3)), Some(&id(4))), Relation::Divergiu);
        assert_eq!(g.relation(Some(&id(1)), Some(&id(5))), Relation::Divergiu);
        assert_eq!(g.relation(Some(&id(2)), Some(&id(6))), Relation::Igual);
        assert_eq!(g.relation(None, Some(&id(1))), Relation::Avanco);
        assert_eq!(g.relation(Some(&id(1)), None), Relation::Envio);
    }
}
