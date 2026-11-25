//! Testes de propriedade: históricos aleatórios em 3 máquinas.
//!
//! Invariantes (critérios da fase 1 do roadmap):
//! 1. Nenhuma restauração sem cópia: sempre que o disco é sobrescrito, o conteúdo anterior já
//!    está num snapshot.
//! 2. Nenhum snapshot perdido: depois de convergir, todo snapshot criado é ancestral do head
//!    final de todas as máquinas.
//! 3. Convergência: depois de rodadas de sync (resolvendo conflitos), todas as máquinas têm o
//!    mesmo conteúdo.
//!
//! O número de casos padrão é 10 000; `PROPTEST_CASES` muda isso.

use proptest::prelude::*;
use relicario_engine::{Action, Graph, Node, PlanInput, plan};
use relicario_model::{Hash, MachineId, SnapshotId};
use std::collections::HashSet;

const N: usize = 3;

#[derive(Debug, Clone)]
enum Op {
    Jogar(usize),
    Sync(usize),
    Resolver(usize, usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (0..N).prop_map(Op::Jogar),
        4 => (0..N).prop_map(Op::Sync),
        1 => (0..N, 0..4usize).prop_map(|(m, i)| Op::Resolver(m, i)),
    ]
}

struct Maquina {
    id: MachineId,
    disco: Option<Hash>,
    head: Option<SnapshotId>,
    pendentes: Vec<SnapshotId>,
    conflito: Option<Vec<SnapshotId>>,
}

struct Mundo {
    /// Snapshots enviados ao repositório compartilhado.
    repo: Graph,
    /// Todos os snapshots, inclusive os ainda não enviados.
    todos: Graph,
    publicados: std::collections::BTreeMap<usize, SnapshotId>,
    maquinas: Vec<Maquina>,
    contador: u64,
    relogio: i64,
}

impl Mundo {
    fn new() -> Self {
        Mundo {
            repo: Graph::new(),
            todos: Graph::new(),
            publicados: Default::default(),
            maquinas: (0..N)
                .map(|i| Maquina {
                    id: MachineId::new(format!("m{i}")),
                    disco: None,
                    head: None,
                    pendentes: vec![],
                    conflito: None,
                })
                .collect(),
            contador: 0,
            relogio: 0,
        }
    }

    fn novo_hash(&mut self, tag: u8) -> Hash {
        self.contador += 1;
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&self.contador.to_le_bytes());
        b[31] = tag;
        Hash::from_bytes(b)
    }

    fn conteudo_guardado(&self, c: Hash) -> bool {
        self.todos.ids().any(|id| self.todos.content(id) == Some(c))
    }

    /// Sobrescreve o disco conferindo a invariante 1.
    fn sobrescrever(&mut self, m: usize, novo: Option<Hash>) {
        if let Some(atual) = self.maquinas[m].disco
            && Some(atual) != novo
        {
            assert!(
                self.conteudo_guardado(atual),
                "restauração sem cópia na máquina {m}"
            );
        }
        self.maquinas[m].disco = novo;
    }

    fn snapshot(&mut self, m: usize, parents: Vec<SnapshotId>, content: Hash) -> SnapshotId {
        let id = self.novo_hash(1);
        self.relogio += 1;
        let node = Node {
            parents,
            content,
            machine: self.maquinas[m].id.clone(),
            created_at: self.relogio,
        };
        self.todos.insert(id, node);
        self.maquinas[m].pendentes.push(id);
        self.maquinas[m].head = Some(id);
        id
    }

    fn visao(&self, m: usize) -> Graph {
        let mut g = self.repo.clone();
        for id in &self.maquinas[m].pendentes {
            g.insert(*id, self.todos.get(id).unwrap().clone());
        }
        g
    }

    fn enviar(&mut self, m: usize) {
        for id in std::mem::take(&mut self.maquinas[m].pendentes) {
            self.repo.insert(id, self.todos.get(&id).unwrap().clone());
        }
        if let Some(h) = self.maquinas[m].head {
            self.publicados.insert(m, h);
        }
    }

    fn jogar(&mut self, m: usize) {
        let c = self.novo_hash(2);
        // O jogo grava por cima: isso não é restauração, então a invariante 1 não se aplica.
        self.maquinas[m].disco = Some(c);
    }

    fn sync(&mut self, m: usize) {
        let g = self.visao(m);
        let remotes = self
            .publicados
            .iter()
            .filter(|(k, _)| **k != m)
            .map(|(k, v)| (self.maquinas[*k].id.clone(), *v))
            .collect();
        let p = plan(&PlanInput {
            graph: &g,
            disk: self.maquinas[m].disco,
            local_head: self.maquinas[m].head,
            remotes,
        });
        if p.dirty {
            let parents = self.maquinas[m].head.into_iter().collect();
            let c = self.maquinas[m].disco.expect("sujo sem disco");
            self.snapshot(m, parents, c);
        }
        self.maquinas[m].conflito = None;
        match p.action {
            Action::Nada | Action::Enviar => {}
            Action::Adotar { to } => {
                assert_eq!(g.content(&to), self.maquinas[m].disco);
                self.maquinas[m].head = Some(to);
            }
            Action::Avancar { to, .. } => {
                assert!(!p.dirty, "avanço com disco sujo");
                if let Some(h) = self.maquinas[m].head {
                    assert_eq!(
                        g.is_ancestor(&h, &to),
                        relicario_engine::graph::Ancestry::Sim,
                        "avanço para alvo que não descende do head local"
                    );
                }
                let c = g.content(&to);
                self.sobrescrever(m, c);
                self.maquinas[m].head = Some(to);
            }
            Action::Conflito { tips } => {
                let mut ids: Vec<SnapshotId> = tips.into_iter().map(|(_, id)| id).collect();
                if let Some(h) = self.maquinas[m].head
                    && !ids.contains(&h)
                {
                    ids.insert(0, h);
                }
                self.maquinas[m].conflito = Some(ids);
            }
        }
        self.enviar(m);
    }

    fn resolver(&mut self, m: usize, escolha: usize) {
        let Some(mut tips) = self.maquinas[m].conflito.take() else {
            return;
        };
        // O jogo pode ter gravado depois do conflito: cópia de segurança antes de resolver.
        if let Some(d) = self.maquinas[m].disco
            && !self.conteudo_guardado(d)
        {
            let parents = self.maquinas[m].head.into_iter().collect();
            let novo = self.snapshot(m, parents, d);
            tips.retain(|t| {
                Some(*t)
                    != self
                        .todos
                        .get(&novo)
                        .and_then(|n| n.parents.first().copied())
            });
            tips.insert(0, novo);
        }
        let g = self.visao(m);
        let alvo = tips[escolha % tips.len()];
        let c = g.content(&alvo).unwrap();
        self.snapshot(m, tips, c);
        self.sobrescrever(m, Some(c));
        self.enviar(m);
    }

    fn convergir(&mut self) {
        for _ in 0..20 {
            for m in 0..N {
                self.sync(m);
                if self.maquinas[m].conflito.is_some() {
                    self.resolver(m, 0);
                }
            }
            let conteudos: HashSet<_> = self.maquinas.iter().map(|x| x.disco).collect();
            if conteudos.len() == 1 && self.maquinas.iter().all(|x| x.conflito.is_none()) {
                return;
            }
        }
        panic!(
            "não convergiu: discos {:?}",
            self.maquinas.iter().map(|x| x.disco).collect::<Vec<_>>()
        );
    }
}

fn casos() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: casos(), ..ProptestConfig::default() })]

    #[test]
    fn historicos_aleatorios_em_tres_maquinas(ops in prop::collection::vec(op(), 0..60)) {
        let mut w = Mundo::new();
        for o in ops {
            match o {
                Op::Jogar(m) => w.jogar(m),
                Op::Sync(m) => w.sync(m),
                Op::Resolver(m, i) => w.resolver(m, i),
            }
        }
        w.convergir();
        // Invariante 2: todo snapshot está na história de algum head final, e tudo já foi
        // enviado ao repositório compartilhado.
        let mut anc = HashSet::new();
        for m in &w.maquinas {
            prop_assert!(m.pendentes.is_empty());
            if let Some(h) = m.head {
                anc.extend(w.todos.ancestors(&h));
            }
        }
        for id in w.todos.ids() {
            prop_assert!(anc.contains(id), "snapshot {:?} ficou fora da história", id);
            prop_assert!(w.repo.contains(id));
        }
    }
}
