//! Decide o que fazer com um jogo.

use crate::graph::{Ancestry, Graph};
use relicario_model::{Hash, MachineId, Relation, SnapshotId};

/// Tudo que o motor precisa para decidir.
#[derive(Debug, Clone)]
pub struct PlanInput<'a> {
    pub graph: &'a Graph,
    /// Conteúdo atual no disco ([`relicario_model::Tree::content_hash`]); `None` se não há
    /// nenhum arquivo de save.
    pub disk: Option<Hash>,
    /// Head desta máquina.
    pub local_head: Option<SnapshotId>,
    /// Heads das outras máquinas.
    pub remotes: Vec<(MachineId, SnapshotId)>,
}

/// O que fazer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Tudo igual.
    Nada,
    /// O local está à frente: enviar (depois do snapshot, se `dirty`).
    Enviar,
    /// Outra máquina está à frente: restaurar `to`. Se `dirty`, o snapshot de segurança vem
    /// antes, sempre.
    Avancar { to: SnapshotId, from: MachineId },
    /// O disco já tem o mesmo conteúdo de `to`: só mover o head, sem tocar nos arquivos.
    Adotar { to: SnapshotId },
    /// Duas ou mais pontas mudaram de forma independente. Nada é restaurado.
    Conflito { tips: Vec<(MachineId, SnapshotId)> },
}

/// Decisão para um jogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// O disco mudou desde o head local: é preciso tirar um snapshot antes de qualquer coisa.
    pub dirty: bool,
    pub action: Action,
}

/// Calcula o plano. Regra de ouro: nunca devolve [`Action::Avancar`] para um alvo que não
/// descende do head local, e sempre marca `dirty` quando o disco tem algo que não está no
/// repositório.
pub fn plan(input: &PlanInput<'_>) -> Plan {
    let g = input.graph;
    let head_content = input.local_head.as_ref().and_then(|h| g.content(h));
    let dirty = match (&input.local_head, input.disk) {
        (None, None) => false,
        (None, Some(_)) => true,
        (Some(_), disk) => disk != head_content,
    };

    // Pontas remotas que trazem algo diferente do que temos.
    let mut avancos: Vec<(MachineId, SnapshotId)> = Vec::new();
    let mut divergentes: Vec<(MachineId, SnapshotId)> = Vec::new();
    let mut algum_envio = false;
    // Heads remotos que já estão contidos na história de outro head remoto não trazem nada
    // novo: ficar só com a fronteira evita conflitos falsos depois de um merge.
    let fronteira: Vec<&(MachineId, SnapshotId)> = input
        .remotes
        .iter()
        .filter(|(_, r)| {
            !input
                .remotes
                .iter()
                .any(|(_, r2)| r2 != r && g.is_ancestor(r, r2) == Ancestry::Sim)
        })
        .collect();
    for (m, r) in fronteira {
        // Igual ao disco atual: nada a fazer com essa máquina.
        if input.disk.is_some() && g.content(r) == input.disk {
            if dirty {
                // O disco já está no conteúdo dela: basta adotar o head.
                avancos.push((m.clone(), *r));
            }
            continue;
        }
        match g.relation(input.local_head.as_ref(), Some(r)) {
            Relation::Igual => {}
            Relation::Envio => algum_envio = true,
            Relation::Avanco => avancos.push((m.clone(), *r)),
            Relation::Divergiu => divergentes.push((m.clone(), *r)),
        }
    }

    let local_tip = || {
        input
            .local_head
            .map(|h| (MachineId::new("<local>"), h))
            .into_iter()
            .collect::<Vec<_>>()
    };

    if !divergentes.is_empty() {
        let mut tips = local_tip();
        tips.extend(divergentes);
        tips.extend(avancos);
        return Plan {
            dirty,
            action: Action::Conflito { tips: dedup(tips) },
        };
    }

    if !avancos.is_empty() {
        // Escolher a ponta que descende de todas as outras.
        let melhor = avancos.iter().find(|(_, cand)| {
            avancos.iter().all(|(_, outro)| {
                g.is_ancestor(outro, cand) == Ancestry::Sim || g.content(outro) == g.content(cand)
            })
        });
        let Some((m, alvo)) = melhor.cloned() else {
            let mut tips = local_tip();
            tips.extend(avancos);
            return Plan {
                dirty,
                action: Action::Conflito { tips: dedup(tips) },
            };
        };
        if input.disk.is_some() && g.content(&alvo) == input.disk {
            return Plan {
                dirty: false,
                action: Action::Adotar { to: alvo },
            };
        }
        if dirty
            && let Some(d) = input.disk
            && g.ancestors(&alvo).iter().any(|a| g.content(a) == Some(d))
        {
            // O conteúdo do disco já está guardado na história do alvo: nada se perde.
            return Plan {
                dirty: false,
                action: Action::Avancar { to: alvo, from: m },
            };
        }
        if dirty {
            // Trabalho novo no disco e trabalho novo em outra máquina: conflito.
            let mut tips = local_tip();
            tips.push((m, alvo));
            return Plan {
                dirty,
                action: Action::Conflito { tips: dedup(tips) },
            };
        }
        return Plan {
            dirty,
            action: Action::Avancar { to: alvo, from: m },
        };
    }

    if dirty || algum_envio {
        return Plan {
            dirty,
            action: Action::Enviar,
        };
    }
    Plan {
        dirty,
        action: Action::Nada,
    }
}

fn dedup(mut tips: Vec<(MachineId, SnapshotId)>) -> Vec<(MachineId, SnapshotId)> {
    let mut vistos = std::collections::HashSet::new();
    tips.retain(|(_, id)| vistos.insert(*id));
    tips
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Node;

    fn id(n: u8) -> SnapshotId {
        Hash::from_bytes([n; 32])
    }
    fn c(n: u8) -> Hash {
        Hash::from_bytes([n + 100; 32])
    }
    fn g() -> Graph {
        let mut g = Graph::new();
        let mut add = |i: u8, p: &[u8]| {
            g.insert(
                id(i),
                Node {
                    parents: p.iter().map(|x| id(*x)).collect(),
                    content: c(i),
                    machine: MachineId::new("x"),
                    created_at: i as i64,
                },
            )
        };
        add(1, &[]);
        add(2, &[1]);
        add(3, &[2]);
        add(4, &[2]);
        g
    }
    fn m(s: &str) -> MachineId {
        MachineId::new(s)
    }

    #[test]
    fn avanca_quando_o_outro_esta_a_frente() {
        let g = g();
        let p = plan(&PlanInput {
            graph: &g,
            disk: Some(c(1)),
            local_head: Some(id(1)),
            remotes: vec![(m("b"), id(2)), (m("c"), id(3))],
        });
        assert_eq!(
            p,
            Plan {
                dirty: false,
                action: Action::Avancar {
                    to: id(3),
                    from: m("c")
                }
            }
        );
    }

    #[test]
    fn disco_sujo_mais_avanco_vira_conflito() {
        let g = g();
        let p = plan(&PlanInput {
            graph: &g,
            disk: Some(c(50)),
            local_head: Some(id(1)),
            remotes: vec![(m("b"), id(3))],
        });
        assert!(p.dirty);
        assert!(matches!(p.action, Action::Conflito { .. }));
    }

    #[test]
    fn duas_pontas_remotas_divergentes() {
        let g = g();
        let p = plan(&PlanInput {
            graph: &g,
            disk: Some(c(2)),
            local_head: Some(id(2)),
            remotes: vec![(m("b"), id(3)), (m("c"), id(4))],
        });
        assert!(matches!(p.action, Action::Conflito { .. }));
    }

    #[test]
    fn envia_e_adota() {
        let g = g();
        let p = plan(&PlanInput {
            graph: &g,
            disk: Some(c(3)),
            local_head: Some(id(3)),
            remotes: vec![(m("b"), id(2))],
        });
        assert_eq!(p.action, Action::Enviar);
        let p = plan(&PlanInput {
            graph: &g,
            disk: Some(c(3)),
            local_head: Some(id(1)),
            remotes: vec![(m("b"), id(3))],
        });
        assert_eq!(
            p,
            Plan {
                dirty: false,
                action: Action::Adotar { to: id(3) }
            }
        );
        let p = plan(&PlanInput {
            graph: &g,
            disk: None,
            local_head: None,
            remotes: vec![],
        });
        assert_eq!(p.action, Action::Nada);
        let p = plan(&PlanInput {
            graph: &g,
            disk: Some(c(9)),
            local_head: None,
            remotes: vec![],
        });
        assert_eq!(
            p,
            Plan {
                dirty: true,
                action: Action::Enviar
            }
        );
    }
}
