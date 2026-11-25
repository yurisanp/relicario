//! Ciclo do jogo: antes de abrir, depois de fechar e a conferência periódica. Usado pelo daemon
//! e pelo `relicario run` quando o daemon não está rodando.

use crate::config::Config;
use crate::games::Game;
use relicario_engine::Action;
use relicario_model::{GameId, MachineId, SnapshotId, SnapshotReason, Suspicion};
use relicario_store::ops::{self, Local, SnapshotOutcome};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// O que aconteceu com um jogo numa sincronização.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SyncOutcome {
    EmDia,
    /// Jogo em modo só histórico: outra máquina tem save mais novo, mas quem traz é a loja.
    SoHistorico {
        from: String,
    },
    /// Snapshot novo desta máquina.
    Guardado {
        snapshot: String,
    },
    /// Snapshot guardado, mas suspeito: não virou head.
    Suspeito {
        snapshot: String,
        reasons: Vec<String>,
    },
    /// Restaurado de outra máquina.
    Restaurado {
        snapshot: String,
        from: String,
        safety: Option<String>,
    },
    /// Outra máquina está à frente, mas o jogo está aberto aqui: nada foi restaurado.
    AdiadoJogoAberto {
        snapshot: String,
        from: String,
    },
    /// Conflito: nada foi restaurado.
    Conflito {
        tips: Vec<(String, String)>,
    },
}

impl SyncOutcome {
    /// Frase para o usuário.
    pub fn describe(&self, title: &str) -> String {
        match self {
            SyncOutcome::EmDia => format!("{title}: em dia"),
            SyncOutcome::Guardado { snapshot } => format!("{title}: snapshot {snapshot}"),
            SyncOutcome::Suspeito { snapshot, reasons } => {
                format!(
                    "{title}: snapshot {snapshot} parece corrompido e não foi enviado ({})",
                    reasons.join("; ")
                )
            }
            SyncOutcome::Restaurado {
                snapshot,
                from,
                safety,
            } => match safety {
                Some(s) => format!("{title}: trouxe {snapshot} de {from} (cópia de segurança {s})"),
                None => format!("{title}: trouxe {snapshot} de {from}"),
            },
            SyncOutcome::AdiadoJogoAberto { from, .. } => {
                format!(
                    "{title}: {from} tem um save mais novo, mas o jogo está aberto aqui; nada foi restaurado"
                )
            }
            SyncOutcome::SoHistorico { from } => {
                format!("{title}: só histórico (a nuvem da loja traz o save de {from})")
            }
            SyncOutcome::Conflito { tips } => {
                let t: Vec<String> = tips.iter().map(|(m, s)| format!("{m} ({s})")).collect();
                format!(
                    "{title}: CONFLITO entre {}; nada foi restaurado",
                    t.join(", ")
                )
            }
        }
    }
}

pub fn describe_suspicion(s: &Suspicion) -> String {
    match s {
        Suspicion::Shrunk { from, to } => format!("encolheu de {from} para {to} bytes"),
        Suspicion::Emptied => "o save ficou vazio".into(),
        Suspicion::Zeroed { path } => format!("{path} virou só zeros"),
        Suspicion::TypeChanged { path } => format!("{path} mudou de formato"),
    }
}

fn machine_name(l: &Local<'_>, m: &MachineId) -> String {
    if *m == l.machine {
        return l.machine_name.clone();
    }
    l.repo
        .heads_of(m)
        .ok()
        .flatten()
        .map(|h| h.machine_name)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| m.to_string())
}

/// Sincroniza um jogo: guarda o que mudou e, se `allow_restore`, traz o que está mais novo em
/// outra máquina. Nunca restaura com conflito nem sem cópia de segurança.
pub fn sync_game(
    cfg: &Config,
    game: &Game,
    reason: SnapshotReason,
    allow_restore: bool,
) -> Result<SyncOutcome, String> {
    let mut repo = cfg.open_repo()?;
    let spec = game.config.spec()?;
    let mut l = Local {
        repo: &mut repo,
        machine: cfg.machine(),
        machine_name: cfg.machine_name.clone(),
    };
    let (p, _) = ops::plan(&l, &spec).map_err(|e| e.to_string())?;
    let mut saved = None;
    if p.dirty && !matches!(p.action, Action::Avancar { .. } | Action::Adotar { .. }) {
        match ops::take_snapshot(&mut l, &spec, reason, false).map_err(|e| e.to_string())? {
            SnapshotOutcome::Created(id) => saved = Some(id),
            SnapshotOutcome::Suspect(id, why) => {
                return Ok(SyncOutcome::Suspeito {
                    snapshot: id.short(),
                    reasons: why.iter().map(describe_suspicion).collect(),
                });
            }
            SnapshotOutcome::Unchanged(_) => {}
        }
    }
    Ok(match p.action {
        Action::Nada | Action::Enviar => match saved {
            Some(id) => SyncOutcome::Guardado {
                snapshot: id.short(),
            },
            None => SyncOutcome::EmDia,
        },
        Action::Adotar { to } => {
            ops::adopt(&mut l, &spec.id, to).map_err(|e| e.to_string())?;
            SyncOutcome::EmDia
        }
        Action::Avancar { from, .. } if game.history_only(cfg) => SyncOutcome::SoHistorico {
            from: machine_name(&l, &from),
        },
        Action::Conflito { .. } if game.history_only(cfg) => match saved {
            Some(id) => SyncOutcome::Guardado {
                snapshot: id.short(),
            },
            None => SyncOutcome::EmDia,
        },
        Action::Avancar { to, from } => {
            let from = machine_name(&l, &from);
            if !allow_restore {
                return Ok(SyncOutcome::AdiadoJogoAberto {
                    snapshot: to.short(),
                    from,
                });
            }
            let out = ops::restore(&mut l, &spec, to).map_err(|e| e.to_string())?;
            SyncOutcome::Restaurado {
                snapshot: to.short(),
                from,
                safety: out.safety.map(|s| s.short()),
            }
        }
        Action::Conflito { tips } => SyncOutcome::Conflito {
            tips: tips
                .iter()
                .map(|(m, s)| (machine_name(&l, m), s.short()))
                .collect(),
        },
    })
}

/// Antes de abrir o jogo: sincroniza (pode restaurar, o jogo ainda não está aberto), marca o
/// jogo como aberto nesta máquina e avisa se ele está aberto em outra.
pub fn before_launch(cfg: &Config, game: &Game) -> Result<Vec<String>, String> {
    let mut msgs = Vec::new();
    // Par direto primeiro, com limite curto: o jogo não pode esperar.
    for p in crate::p2p::sync_peers(cfg, &[], Duration::from_secs(3)) {
        if !p.ok {
            msgs.push(p.message);
        }
    }
    let out = sync_game(cfg, game, SnapshotReason::Periodic, true)?;
    msgs.push(out.describe(game.title()));
    if let SyncOutcome::Conflito { .. } = out {
        notify(
            game.title(),
            "Conflito de save: o jogo vai abrir com o save desta máquina. Resolva depois no Relicário.",
        );
    }
    if let SyncOutcome::Suspeito { .. } = out {
        notify(
            game.title(),
            "O save desta máquina parece corrompido e não foi enviado.",
        );
    }
    let mut repo = cfg.open_repo()?;
    let mut l = Local {
        repo: &mut repo,
        machine: cfg.machine(),
        machine_name: cfg.machine_name.clone(),
    };
    let id = GameId::new(game.id());
    let now = relicario_store::time::now();
    for (m, _) in ops::open_elsewhere(&l, &id, now).map_err(|e| e.to_string())? {
        let t = format!("{} está aberto em {m}", game.title());
        notify("Jogo aberto em outra máquina", &t);
        msgs.push(t);
    }
    ops::set_running(&mut l, &id, Some(now)).map_err(|e| e.to_string())?;
    Ok(msgs)
}

/// Quanto esperar o jogo terminar de gravar.
pub fn settle_quiet() -> Duration {
    std::env::var("RELICARIO_SETTLE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(5))
}

/// Depois que o jogo fecha: espera terminar de gravar, guarda, soma tempo de jogo e tira a marca
/// de aberto.
pub fn after_exit(
    cfg: &Config,
    game: &Game,
    started_at: Option<i64>,
) -> Result<Vec<String>, String> {
    let spec = game.config.spec()?;
    let sources = spec.sources.clone();
    let quiet = settle_quiet();
    relicario_watch::settle::wait_settled(
        || {
            sources
                .iter()
                .flat_map(|s| {
                    relicario_store::files::list_files(s)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(_, p)| p)
                })
                .collect()
        },
        quiet,
        quiet * 24,
        (quiet / 5).max(Duration::from_millis(50)),
    );
    let out = sync_game(cfg, game, SnapshotReason::GameExit, true)?;
    if let SyncOutcome::Suspeito { .. } = out {
        notify(
            game.title(),
            "O save parece corrompido e não foi enviado. Veja o histórico no Relicário.",
        );
    }
    if let SyncOutcome::Conflito { .. } = out {
        notify(
            game.title(),
            "Conflito de save com outra máquina. As duas versões estão guardadas.",
        );
    }
    let mut repo = cfg.open_repo()?;
    let mut l = Local {
        repo: &mut repo,
        machine: cfg.machine(),
        machine_name: cfg.machine_name.clone(),
    };
    let id = GameId::new(game.id());
    let now = relicario_store::time::now();
    if let Some(t) = started_at
        && now > t
    {
        ops::add_playtime(&mut l, &id, (now - t) as u64).map_err(|e| e.to_string())?;
    }
    ops::set_running(&mut l, &id, None).map_err(|e| e.to_string())?;
    let mut msgs = vec![out.describe(game.title())];
    for p in crate::p2p::sync_peers(cfg, &[], Duration::from_secs(3)) {
        msgs.push(p.message);
    }
    Ok(msgs)
}

pub fn notify(title: &str, body: &str) {
    relicario_platform::notify::notify(title, body);
}

/// Resolve um snapshot pelo id curto.
pub fn short(id: &SnapshotId) -> String {
    id.short()
}
