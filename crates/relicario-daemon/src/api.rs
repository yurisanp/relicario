//! O que a interface pede ao daemon: telas prontas em JSON e ações. Tudo passa pelo daemon
//! (pelo socket ou, sem daemon de fora, por um daemon dentro do próprio app), que serializa as
//! operações no repositório com as do ciclo dos jogos.

use crate::config::{CloudConfig, Config};
use crate::games::Game;
use crate::lifecycle::SyncOutcome;
use crate::server::Daemon;
use base64::Engine as _;
use relicario_catalog::art::{ArtChoice, ArtKind};
use relicario_catalog::rules::{Rule, SyncMode};
use relicario_engine::Action;
use relicario_model::{GameId, Hash, MachineId, Snapshot, SnapshotId, SnapshotReason};
use relicario_store::Repository;
use relicario_store::ops::{self, Local};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Pedido da interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum Call {
    /// Biblioteca, máquinas, nuvens e contadores.
    Overview,
    /// Detalhe de um jogo com a árvore de snapshots.
    Game {
        id: String,
    },
    /// Arquivos e o que o leitor de save achou num snapshot.
    Snapshot {
        game: String,
        snapshot: String,
    },
    Activity {
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        game: Option<String>,
    },
    /// Esta máquina, pares, máquinas na rede e nuvens.
    Devices,
    Settings,
    Restore {
        game: String,
        snapshot: String,
    },
    Pin {
        snapshot: String,
        name: String,
    },
    Unpin {
        snapshot: String,
    },
    /// Resolve um conflito ficando com `keep`. Com `keep_both`, as outras versões ficam fixadas
    /// com o nome da máquina, para não serem apagadas pela retenção.
    Resolve {
        game: String,
        keep: String,
        #[serde(default)]
        keep_both: bool,
    },
    SetMode {
        game: String,
        mode: Option<SyncMode>,
    },
    SetIgnored {
        game: String,
        ignored: bool,
    },
    SaveSettings {
        settings: SettingsIn,
    },
    /// Arte em `data:` URL.
    Art {
        game: String,
        kind: ArtKind,
    },
    ArtChoices {
        game: String,
        kind: ArtKind,
    },
    ArtSet {
        game: String,
        kind: ArtKind,
        choice: ArtChoice,
    },
    ArtReset {
        game: String,
        kind: ArtKind,
    },
    /// Troca a arte por uma imagem escolhida pelo usuário.
    ArtFile {
        game: String,
        kind: ArtKind,
        path: PathBuf,
    },
    /// Exporta um snapshot num `.zip`.
    ExportZip {
        game: String,
        snapshot: String,
        dest: PathBuf,
    },
    /// Entra no pareamento aberto em outra máquina.
    PairJoin {
        code: String,
        #[serde(default)]
        addr: Option<String>,
    },
    PeerRemove {
        machine: String,
    },
    CloudAdd {
        cloud: CloudConfig,
        #[serde(default)]
        secret: Option<String>,
        #[serde(default)]
        password: Option<String>,
    },
    CloudRemove {
        name: String,
    },
    /// Aplica a retenção e libera espaço no repositório desta máquina.
    Prune,
    CloudSync,
    CloudCheck {
        name: String,
    },
}

/// Ajustes que a interface pode mudar.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsIn {
    #[serde(default)]
    pub machine_name: Option<String>,
    #[serde(default)]
    pub auto_detect: Option<bool>,
    #[serde(default)]
    pub periodic_secs: Option<u64>,
    /// Vazio apaga a chave.
    #[serde(default)]
    pub steamgriddb_key: Option<String>,
    /// 0 desliga a sincronização direta.
    #[serde(default)]
    pub p2p_port: Option<u16>,
    /// Limite de banda por nuvem (KiB/s, 0 sem limite).
    #[serde(default)]
    pub cloud_limits: BTreeMap<String, u32>,
    #[serde(default)]
    pub retention: Option<crate::config::Retention>,
}

type R<T> = Result<T, String>;

fn parse_id(s: &str) -> R<SnapshotId> {
    s.parse::<Hash>()
        .map_err(|_| format!("snapshot inválido: {s}"))
}

fn reason(r: SnapshotReason) -> &'static str {
    match r {
        SnapshotReason::Manual => "manual",
        SnapshotReason::GameExit => "jogo_fechou",
        SnapshotReason::Periodic => "periodico",
        SnapshotReason::BeforeRestore => "antes_de_restaurar",
        SnapshotReason::Merge => "resolucao",
        SnapshotReason::Import => "importado",
        SnapshotReason::Restored => "restaurado",
    }
}

fn data_url(mime: &str, data: &[u8]) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(data)
    )
}

fn mime_of(p: &std::path::Path) -> &'static str {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        _ => "image/jpeg",
    }
}

/// Nome de cada máquina que aparece no repositório.
fn machine_names(repo: &Repository, cfg: &Config) -> BTreeMap<String, String> {
    let mut m: BTreeMap<String, String> = repo
        .heads()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|h| {
            let id = h.machine?.to_string();
            let name = if h.machine_name.is_empty() {
                id.clone()
            } else {
                h.machine_name
            };
            Some((id, name))
        })
        .collect();
    for p in &cfg.peers {
        m.entry(p.machine.clone()).or_insert_with(|| p.name.clone());
    }
    m.insert(cfg.machine_id.clone(), cfg.machine_name.clone());
    m
}

fn rule_of<'a>(cfg: &'a Config, game: &str) -> Option<&'a Rule> {
    cfg.rules.iter().rev().find(|r| r.game == game)
}

fn rule_mut<'a>(cfg: &'a mut Config, game: &str) -> &'a mut Rule {
    if let Some(i) = cfg.rules.iter().rposition(|r| r.game == game) {
        return &mut cfg.rules[i];
    }
    cfg.rules.push(Rule {
        game: game.to_string(),
        exclude: vec![],
        ignore: false,
        roots: vec![],
        mode: None,
    });
    cfg.rules.last_mut().expect("acabou de entrar")
}

fn outcome_json(out: &SyncOutcome, title: &str) -> Value {
    json!({
        "kind": out.kind(),
        "text": out.describe(title),
        "detail": out,
    })
}

impl Daemon {
    /// Atende um pedido da interface.
    pub fn api(&self, call: Call) -> R<Value> {
        match call {
            Call::Overview => self.overview(),
            Call::Game { id } => self.game_detail(&id),
            Call::Snapshot { game, snapshot } => self.snapshot_detail(&game, &snapshot),
            Call::Activity { limit, game } => Ok(json!(crate::activity::recent(
                limit.unwrap_or(100).min(crate::activity::KEEP),
                game.as_deref()
            ))),
            Call::Devices => self.devices(),
            Call::Settings => self.settings(),
            Call::Restore { game, snapshot } => self.restore(&game, &snapshot),
            Call::Pin { snapshot, name } => self.with_local(|l| {
                ops::pin(l, parse_id(&snapshot)?, name.clone()).map_err(|e| e.to_string())?;
                crate::activity::record("fixado", None, format!("fixado como \"{name}\""));
                Ok(json!({"ok": true}))
            }),
            Call::Unpin { snapshot } => self.with_local(|l| {
                let had = ops::unpin(l, parse_id(&snapshot)?).map_err(|e| e.to_string())?;
                Ok(json!({ "ok": had }))
            }),
            Call::Resolve {
                game,
                keep,
                keep_both,
            } => self.resolve(&game, &keep, keep_both),
            Call::SetMode { game, mode } => self.edit_config(|c| {
                rule_mut(c, &game).mode = mode;
                crate::activity::clear_last(&game);
                Ok(())
            }),
            Call::SetIgnored { game, ignored } => self.edit_config(|c| {
                rule_mut(c, &game).ignore = ignored;
                Ok(())
            }),
            Call::SaveSettings { settings } => self.save_settings(settings),
            Call::Art { game, kind } => self.art(&game, kind),
            Call::ArtChoices { game, kind } => {
                let (cfg, g) = self.game(&game)?;
                let list = crate::games::art_cache(&cfg)
                    .choices(&g.art_query(), kind, 24)
                    .map_err(|e| e.to_string())?;
                Ok(json!({ "has_key": cfg.steamgriddb_key.is_some(), "choices": list }))
            }
            Call::ArtSet { game, kind, choice } => {
                let (cfg, _) = self.game(&game)?;
                crate::games::art_cache(&cfg)
                    .set_choice(&game, kind, &choice)
                    .map_err(|e| e.to_string())?;
                self.art(&game, kind)
            }
            Call::ArtFile { game, kind, path } => {
                let (cfg, _) = self.game(&game)?;
                crate::games::art_cache(&cfg)
                    .set_override(&game, kind, &path)
                    .map_err(|e| e.to_string())?;
                self.art(&game, kind)
            }
            Call::ArtReset { game, kind } => {
                let (cfg, _) = self.game(&game)?;
                crate::games::art_cache(&cfg).clear(&game, kind);
                self.art(&game, kind)
            }
            Call::ExportZip {
                game,
                snapshot,
                dest,
            } => self.export_zip(&game, &snapshot, &dest),
            Call::PairJoin { code, addr } => self.pair_join(&code, addr.as_deref()),
            Call::PeerRemove { machine } => self.edit_config(|c| {
                let before = c.peers.len();
                c.peers
                    .retain(|p| p.machine != machine && !p.name.eq_ignore_ascii_case(&machine));
                if c.peers.len() == before {
                    return Err(format!("nenhuma máquina pareada chamada {machine:?}"));
                }
                Ok(())
            }),
            Call::CloudAdd {
                mut cloud,
                mut secret,
                password,
            } => {
                // Google Drive: entra pelo navegador (espera até 5 minutos).
                if let relicario_transport::CloudSpec::Gdrive {
                    client_id,
                    client_secret,
                    ..
                } = &mut cloud.spec
                    && secret.is_none()
                {
                    let (id, sec, token) = crate::oauth::gdrive_login(|_| {})?;
                    *client_id = id;
                    *client_secret = sec;
                    secret = Some(token);
                }
                let mut cfg = Config::load()?;
                let name = cloud.name.clone();
                let added = crate::cloud::add(&mut cfg, cloud, secret, password)?;
                self.reload(false)?;
                crate::activity::record("nuvem", None, format!("nuvem {name} conectada"));
                self.cloud_dirty.store(true, Ordering::SeqCst);
                Ok(match added {
                    crate::cloud::Added::New { recovery } => {
                        json!({"ok": true, "new": true, "recovery": recovery})
                    }
                    crate::cloud::Added::Joined => json!({"ok": true, "new": false}),
                })
            }
            Call::CloudRemove { name } => self.edit_config(|c| {
                let before = c.clouds.len();
                c.clouds.retain(|x| x.name != name);
                if c.clouds.len() == before {
                    return Err(format!("nenhuma nuvem chamada {name:?}"));
                }
                relicario_platform::secrets::delete(&crate::cloud::secret_name(&name));
                relicario_platform::secrets::delete(&crate::cloud::vault_name(&name));
                Ok(())
            }),
            Call::Prune => {
                let policy = self.snapshot().0.policy();
                self.with_local(|l| {
                    let now = relicario_store::time::now();
                    let forgot = ops::apply_retention(l, policy, now).map_err(|e| e.to_string())?;
                    let m = l.machine.clone();
                    let r = relicario_store::prune::prune(l.repo, &m).map_err(|e| e.to_string())?;
                    Ok(json!({
                        "forgotten": forgot.len(),
                        "packs_removed": r.packs_removed,
                        "bytes_freed": r.bytes_freed,
                    }))
                })
            }
            Call::CloudSync => {
                let cfg = self.snapshot().0;
                let out: Vec<Value> = crate::cloud::sync_all(&cfg)
                    .into_iter()
                    .map(|r| {
                        if r.heads_in {
                            self.remote_changed.store(true, Ordering::SeqCst);
                        }
                        crate::activity::record(
                            if r.ok { "nuvem" } else { "erro" },
                            None,
                            r.message.clone(),
                        );
                        json!({"name": r.name, "ok": r.ok, "auth": r.auth, "message": r.message})
                    })
                    .collect();
                Ok(json!(out))
            }
            Call::CloudCheck { name } => {
                let cfg = self.snapshot().0;
                let c = cfg
                    .clouds
                    .iter()
                    .find(|c| c.name == name)
                    .ok_or_else(|| format!("nenhuma nuvem chamada {name:?}"))?;
                let msgs = crate::cloud::check(c)?;
                Ok(json!({"ok": true, "messages": msgs}))
            }
        }
    }

    fn with_local<T>(&self, f: impl FnOnce(&mut Local<'_>) -> R<T>) -> R<T> {
        let cfg = self.snapshot().0;
        let _w = self.work.lock().expect("lock");
        let mut repo = cfg.open_repo()?;
        let mut l = Local {
            repo: &mut repo,
            machine: cfg.machine(),
            machine_name: cfg.machine_name.clone(),
        };
        f(&mut l)
    }

    fn edit_config(&self, f: impl FnOnce(&mut Config) -> R<()>) -> R<Value> {
        let mut cfg = Config::load()?;
        f(&mut cfg)?;
        cfg.save()?;
        self.reload(false)?;
        Ok(json!({"ok": true}))
    }

    fn running_here(&self) -> BTreeSet<String> {
        let i = self.inner.lock().expect("lock");
        i.watcher
            .running()
            .cloned()
            .chain(i.external.iter().cloned())
            .collect()
    }

    fn overview(&self) -> R<Value> {
        let (cfg, games) = self.snapshot();
        let repo = cfg.open_repo()?;
        let names = machine_names(&repo, &cfg);
        let heads = repo.heads().map_err(|e| e.to_string())?;
        let snaps = repo.valid_snapshots().map_err(|e| e.to_string())?;
        let mut by_game: BTreeMap<String, Vec<&Snapshot>> = BTreeMap::new();
        for (_, s) in &snaps {
            by_game.entry(s.game.to_string()).or_default().push(s);
        }
        let running = self.running_here();
        let now = relicario_store::time::now();
        let last = crate::activity::last();
        let mut seen = BTreeSet::new();
        let mut cards = Vec::new();
        let card = |id: &str, title: &str, g: Option<&Game>| -> Value {
            let list = by_game.get(id).cloned().unwrap_or_default();
            let newest = list.iter().max_by_key(|s| s.created_at);
            let gid = GameId::new(id);
            let playtime: u64 = heads.iter().filter_map(|h| h.playtime.get(&gid)).sum();
            let elsewhere: Vec<String> = heads
                .iter()
                .filter(|h| {
                    h.machine
                        .as_ref()
                        .is_some_and(|m| m.as_str() != cfg.machine_id)
                })
                .filter(|h| {
                    h.running
                        .get(&gid)
                        .is_some_and(|t| now - *t < 12 * 3_600_000)
                })
                .map(|h| h.machine_name.clone())
                .collect();
            let status = last.get(id);
            let state = if running.contains(id) {
                "aberto_aqui"
            } else if let Some((_, o)) = status {
                match o {
                    SyncOutcome::Conflito { .. } => "conflito",
                    SyncOutcome::Suspeito { .. } => "suspeito",
                    SyncOutcome::AdiadoJogoAberto { .. } => "pendente",
                    SyncOutcome::SoHistorico { .. } => "so_historico",
                    _ => "em_dia",
                }
            } else if g.is_none() {
                "nao_instalado"
            } else if list.is_empty() {
                "sem_save"
            } else {
                "em_dia"
            };
            json!({
                "id": id,
                "title": title,
                "installed": g.is_some(),
                "store": g.and_then(|g| g.detected.as_ref()).and_then(|d| d.store.clone()),
                "launcher": g.and_then(|g| g.detected.as_ref()).and_then(|d| d.launcher.clone()),
                "history_only": g.is_some_and(|g| g.history_only(&cfg)),
                "state": state,
                "status": status.map(|(at, o)| json!({"at": at, "outcome": outcome_json(o, title)})),
                "snapshots": list.len(),
                "last_at": newest.map(|s| s.created_at),
                "last_machine": newest.map(|s| names.get(s.machine.as_str()).cloned().unwrap_or_else(|| s.machine.to_string())),
                "size": newest.map(|s| s.size),
                "playtime_ms": playtime,
                "running_elsewhere": elsewhere,
            })
        };
        for g in &games {
            seen.insert(g.id().to_string());
            cards.push(card(g.id(), g.title(), Some(g)));
        }
        // Jogos que só existem no repositório (jogados em outra máquina, não instalados aqui).
        for id in by_game.keys() {
            if !seen.contains(id) && rule_of(&cfg, id).is_none_or(|r| !r.ignore) {
                cards.push(card(id, id, None));
            }
        }
        let conflicts = cards.iter().filter(|c| c["state"] == "conflito").count();
        let machines: Vec<Value> = heads
            .iter()
            .filter_map(|h| {
                let id = h.machine.as_ref()?.to_string();
                Some(json!({
                    "id": id,
                    "name": names.get(&id).cloned().unwrap_or(id.clone()),
                    "me": id == cfg.machine_id,
                    "updated_at": h.updated_at,
                    "running": h.running.keys().map(|g| g.to_string()).collect::<Vec<_>>(),
                }))
            })
            .collect();
        let clouds: Vec<Value> = cfg
            .clouds
            .iter()
            .map(|c| {
                json!({
                    "name": c.name,
                    "describe": crate::cloud::describe(c),
                    "encrypted": c.encrypted,
                })
            })
            .collect();
        Ok(json!({
            "machine": {"id": cfg.machine_id, "name": cfg.machine_name},
            "version": env!("CARGO_PKG_VERSION"),
            "games": cards,
            "conflicts": conflicts,
            "running": running,
            "machines": machines,
            "clouds": clouds,
            "cloud_busy": self.cloud_busy.load(Ordering::SeqCst),
            "peers": cfg.peers.len(),
        }))
    }

    fn game_detail(&self, id: &str) -> R<Value> {
        let (cfg, games) = self.snapshot();
        let g = games.iter().find(|g| g.id() == id);
        let repo = cfg.open_repo()?;
        let names = machine_names(&repo, &cfg);
        let gid = GameId::new(id);
        let snaps = ops::game_snapshots(&repo, &gid).map_err(|e| e.to_string())?;
        let heads = repo.heads().map_err(|e| e.to_string())?;
        let mut pins: BTreeMap<SnapshotId, String> = BTreeMap::new();
        let mut forgotten: BTreeSet<SnapshotId> = BTreeSet::new();
        let mut head_of: BTreeMap<SnapshotId, Vec<String>> = BTreeMap::new();
        for h in &heads {
            pins.extend(h.pins.iter().map(|(k, v)| (*k, v.clone())));
            forgotten.extend(h.forgotten.iter().copied());
            if let (Some(m), Some(head)) = (&h.machine, h.heads.get(&gid)) {
                head_of.entry(head.snapshot).or_default().push(
                    names
                        .get(m.as_str())
                        .cloned()
                        .unwrap_or_else(|| m.to_string()),
                );
            }
        }
        let me = MachineId::new(&cfg.machine_id);
        let local_head = repo
            .heads_of(&me)
            .ok()
            .flatten()
            .and_then(|h| h.heads.get(&gid).map(|x| x.snapshot));
        let mut list: Vec<&(SnapshotId, Snapshot)> = snaps.iter().collect();
        list.sort_by_key(|(_, s)| std::cmp::Reverse(s.created_at));
        let snapshots: Vec<Value> = list
            .iter()
            .map(|(sid, s)| {
                json!({
                    "id": sid.to_hex(),
                    "short": sid.short(),
                    "created_at": s.created_at,
                    "machine": s.machine.to_string(),
                    "machine_name": names.get(s.machine.as_str()).cloned().unwrap_or_else(|| s.machine.to_string()),
                    "reason": reason(s.reason),
                    "size": s.size,
                    "files": s.files,
                    "parents": s.parents.iter().map(|p| p.to_hex()).collect::<Vec<_>>(),
                    "pinned": pins.get(sid),
                    "forgotten": forgotten.contains(sid),
                    "suspicions": s.suspicions.iter().map(crate::lifecycle::describe_suspicion).collect::<Vec<_>>(),
                    "head_of": head_of.get(sid).cloned().unwrap_or_default(),
                    "local_head": local_head == Some(*sid),
                    "meta": s.meta,
                })
            })
            .collect();
        let title = g
            .map(|g| g.title().to_string())
            .unwrap_or_else(|| id.to_string());
        // Plano atual (lê o disco): mostra conflito e o que seria feito.
        let plan = g.and_then(|g| {
            let spec = g.config.spec().ok()?;
            let mut repo = cfg.open_repo().ok()?;
            let l = Local {
                repo: &mut repo,
                machine: cfg.machine(),
                machine_name: cfg.machine_name.clone(),
            };
            let (p, _) = ops::plan(&l, &spec).ok()?;
            let action = match &p.action {
                Action::Nada => json!({"kind": "nada"}),
                Action::Enviar => json!({"kind": "enviar"}),
                Action::Adotar { to } => json!({"kind": "adotar", "to": to.to_hex()}),
                Action::Avancar { to, from } => json!({
                    "kind": "avancar",
                    "to": to.to_hex(),
                    "from": names.get(from.as_str()).cloned().unwrap_or_else(|| from.to_string()),
                }),
                Action::Conflito { tips } => json!({
                    "kind": "conflito",
                    "tips": tips.iter().map(|(m, s)| {
                        let local = m.as_str() == "<local>" || *m == me;
                        json!({
                            "machine": if local { cfg.machine_id.clone() } else { m.to_string() },
                            "name": if local { cfg.machine_name.clone() } else { names.get(m.as_str()).cloned().unwrap_or_else(|| m.to_string()) },
                            "local": local,
                            "snapshot": s.to_hex(),
                        })
                    }).collect::<Vec<_>>(),
                }),
            };
            Some(json!({"dirty": p.dirty, "action": action}))
        });
        let playtime: Vec<Value> = ops::playtime_by_machine(&repo, &gid)
            .unwrap_or_default()
            .into_iter()
            .map(|(m, ms)| json!({"machine": m, "ms": ms}))
            .collect();
        let rule = rule_of(&cfg, id);
        Ok(json!({
            "id": id,
            "title": title,
            "installed": g.is_some(),
            "store": g.and_then(|g| g.detected.as_ref()).and_then(|d| d.store.clone()),
            "store_id": g.and_then(|g| g.detected.as_ref()).and_then(|d| d.store_id.clone()),
            "steam_cloud": g.and_then(|g| g.detected.as_ref()).is_some_and(|d| d.steam_cloud),
            "history_only": g.is_some_and(|g| g.history_only(&cfg)),
            "mode": rule.and_then(|r| r.mode),
            "ignored": rule.is_some_and(|r| r.ignore),
            "excludes": rule.map(|r| r.exclude.clone()).unwrap_or_default(),
            "sources": g.map(|g| g.config.roots.iter().map(|r| json!({
                "root": r.name, "path": r.path, "include": r.include, "exclude": r.exclude,
            })).collect::<Vec<_>>()).unwrap_or_default(),
            "executables": g.map(|g| g.config.executables.clone()).unwrap_or_default(),
            "running_here": self.running_here().contains(id),
            "snapshots": snapshots,
            "plan": plan,
            "playtime": playtime,
            "status": crate::activity::last().get(id).map(|(at, o)| json!({"at": at, "outcome": outcome_json(o, &title)})),
        }))
    }

    fn snapshot_detail(&self, game: &str, snapshot: &str) -> R<Value> {
        let (cfg, games) = self.snapshot();
        let repo = cfg.open_repo()?;
        let sid = parse_id(snapshot)?;
        let s = repo.get_snapshot(&sid).map_err(|e| e.to_string())?;
        if s.game.as_str() != game {
            return Err(format!("snapshot {} não é de {game}", sid.short()));
        }
        let tree = repo.get_tree(&s.tree).map_err(|e| e.to_string())?;
        let title = games
            .iter()
            .find(|g| g.id() == game)
            .map(|g| g.title().to_string())
            .unwrap_or_else(|| game.to_string());
        let infos: Vec<relicario_plugins::FileInfo> = tree
            .files
            .iter()
            .map(|f| {
                relicario_plugins::FileInfo::new(format!("{}/{}", f.root, f.path), f.size, f.mtime)
            })
            .collect();
        let forgotten = repo
            .heads()
            .unwrap_or_default()
            .iter()
            .any(|h| h.forgotten.contains(&sid));
        let meta = if forgotten {
            None
        } else {
            let mut fetch = |p: &str| -> Option<Vec<u8>> {
                let (root, path) = p.split_once('/')?;
                let f = tree.get(root, path)?;
                if f.size > relicario_plugins::MAX_READ {
                    return None;
                }
                relicario_store::files::read_entry(&repo, f).ok()
            };
            relicario_plugins::read(game, &title, &infos, &mut fetch)
        };
        let files: Vec<Value> = tree
            .files
            .iter()
            .map(|f| json!({"root": f.root, "path": f.path, "size": f.size, "mtime": f.mtime}))
            .collect();
        Ok(json!({
            "id": sid.to_hex(),
            "files": files,
            "forgotten": forgotten,
            "meta": meta.map(|m| json!({
                "reader": m.reader,
                "source": m.source,
                "fields": m.fields,
                "thumbnail": m.thumbnail.map(|t| data_url(&t.mime, &t.data)),
            })),
        }))
    }

    fn restore(&self, game: &str, snapshot: &str) -> R<Value> {
        let (cfg, g) = self.game(game)?;
        if self.running_here().contains(game) {
            return Err(format!(
                "{} está aberto: feche o jogo antes de restaurar",
                g.title()
            ));
        }
        let spec = g.config.spec()?;
        let sid = parse_id(snapshot)?;
        let out = self.with_local(|l| ops::rollback(l, &spec, sid).map_err(|e| e.to_string()))?;
        let text = match out.safety {
            Some(s) => format!(
                "{}: voltou para {} (o save de antes virou {})",
                g.title(),
                sid.short(),
                s.short()
            ),
            None => format!("{}: voltou para {}", g.title(), sid.short()),
        };
        crate::activity::record("restaurado", Some(game), text.clone());
        crate::activity::clear_last(game);
        self.cloud_dirty.store(true, Ordering::SeqCst);
        let _ = cfg;
        Ok(json!({
            "ok": true,
            "text": text,
            "safety": out.safety.map(|s| s.to_hex()),
            "written": out.report.written.len(),
        }))
    }

    fn resolve(&self, game: &str, keep: &str, keep_both: bool) -> R<Value> {
        let (cfg, g) = self.game(game)?;
        if self.running_here().contains(game) {
            return Err(format!(
                "{} está aberto: feche o jogo antes de resolver",
                g.title()
            ));
        }
        let spec = g.config.spec()?;
        let keep = parse_id(keep)?;
        let names = machine_names(&cfg.open_repo()?, &cfg);
        let r = self.with_local(|l| {
            let (p, _) = ops::plan(l, &spec).map_err(|e| e.to_string())?;
            // Disco com trabalho novo: guarda antes, para entrar como uma das versões.
            let p = if p.dirty {
                ops::take_snapshot(l, &spec, SnapshotReason::Manual, true)
                    .map_err(|e| e.to_string())?;
                ops::plan(l, &spec).map_err(|e| e.to_string())?.0
            } else {
                p
            };
            let Action::Conflito { tips } = p.action else {
                return Err(format!("{} não está em conflito", g.title()));
            };
            if !tips.iter().any(|(_, s)| *s == keep) {
                return Err("essa versão não é uma das pontas do conflito".into());
            }
            let others: Vec<SnapshotId> = tips
                .iter()
                .map(|(_, s)| *s)
                .filter(|s| *s != keep)
                .collect();
            let (merge, out) = ops::resolve(l, &spec, keep, &others).map_err(|e| e.to_string())?;
            if keep_both {
                for (m, s) in tips.iter().filter(|(_, s)| *s != keep) {
                    let who = if m.as_str() == "<local>" {
                        cfg.machine_name.clone()
                    } else {
                        names
                            .get(m.as_str())
                            .cloned()
                            .unwrap_or_else(|| m.to_string())
                    };
                    ops::pin(l, *s, format!("versão de {who}")).map_err(|e| e.to_string())?;
                }
            }
            Ok((merge, out))
        })?;
        let text = format!(
            "{}: conflito resolvido, ficou a versão {}",
            g.title(),
            keep.short()
        );
        crate::activity::record("resolvido", Some(game), text.clone());
        crate::activity::clear_last(game);
        self.cloud_dirty.store(true, Ordering::SeqCst);
        // Avisa os pares logo, para eles não verem o conflito também.
        let (_, _) = self.sync_peers(Duration::from_secs(3));
        Ok(
            json!({"ok": true, "text": text, "merge": r.0.to_hex(), "safety": r.1.safety.map(|s| s.to_hex())}),
        )
    }

    fn devices(&self) -> R<Value> {
        let cfg = self.snapshot().0;
        let repo = cfg.open_repo()?;
        let heads = repo.heads().unwrap_or_default();
        let fp = crate::p2p::identity()
            .map(|i| i.fingerprint)
            .unwrap_or_default();
        let p2p = self.p2p.lock().expect("lock");
        let discovered = p2p
            .as_ref()
            .map(|p| p.discovered.lock().expect("lock").clone())
            .unwrap_or_default();
        let peers: Vec<Value> = cfg
            .peers
            .iter()
            .map(|p| {
                let h = heads
                    .iter()
                    .find(|h| h.machine.as_ref().is_some_and(|m| m.as_str() == p.machine));
                json!({
                    "machine": p.machine,
                    "name": p.name,
                    "fingerprint": p.fingerprint.chars().take(16).collect::<String>(),
                    "addresses": h.map(|h| h.addresses.clone()).filter(|a| !a.is_empty()).unwrap_or_else(|| p.addresses.clone()),
                    "updated_at": h.map(|h| h.updated_at),
                    "nearby": discovered.iter().any(|(m, _)| *m == p.machine),
                })
            })
            .collect();
        let nearby: Vec<Value> = discovered
            .iter()
            .filter(|(m, _)| *m != cfg.machine_id && !cfg.peers.iter().any(|p| p.machine == *m))
            .map(|(m, a)| json!({"machine": m, "addrs": a.iter().map(|x| x.to_string()).collect::<Vec<_>>()}))
            .collect();
        let clouds: Vec<Value> = cfg
            .clouds
            .iter()
            .map(|c| {
                json!({
                    "name": c.name,
                    "kind": match c.spec {
                        relicario_transport::CloudSpec::S3 { .. } => "s3",
                        relicario_transport::CloudSpec::Webdav { .. } => "webdav",
                        relicario_transport::CloudSpec::Gdrive { .. } => "gdrive",
                        relicario_transport::CloudSpec::Pasta { .. } => "pasta",
                        relicario_transport::CloudSpec::Memory => "memoria",
                    },
                    "describe": crate::cloud::describe(c),
                    "encrypted": c.encrypted,
                    "limit_kbps": c.limit_kbps,
                    "last_check": crate::state::State::load().cloud_checks.get(&c.name),
                })
            })
            .collect();
        Ok(json!({
            "me": {
                "id": cfg.machine_id,
                "name": cfg.machine_name,
                "fingerprint": fp.chars().take(16).collect::<String>(),
                "port": p2p.as_ref().map(|p| p.server.port()),
                "addresses": cfg.port().map(|port| relicario_p2p::local_addresses(port, &relicario_platform::hostname())).unwrap_or_default(),
                "pairing": p2p.as_ref().is_some_and(|p| p.server.pairing_open()),
            },
            "peers": peers,
            "nearby": nearby,
            "clouds": clouds,
            "secrets_backend": relicario_platform::secrets::backend(),
            "gdrive_available": crate::oauth::client().is_some(),
        }))
    }

    fn settings(&self) -> R<Value> {
        let cfg = self.snapshot().0;
        let ignored: Vec<&str> = cfg
            .rules
            .iter()
            .filter(|r| r.ignore)
            .map(|r| r.game.as_str())
            .collect();
        Ok(json!({
            "machine_name": cfg.machine_name,
            "machine_id": cfg.machine_id,
            "repo": cfg.repo_path(),
            "auto_detect": cfg.auto_detect,
            "periodic_secs": cfg.periodic_secs.unwrap_or(3600),
            "has_steamgriddb_key": cfg.steamgriddb_key.is_some(),
            "p2p_port": cfg.port().unwrap_or(0),
            "ignored": ignored,
            "clouds": cfg.clouds.iter().map(|c| json!({"name": c.name, "limit_kbps": c.limit_kbps})).collect::<Vec<_>>(),
            "retention": cfg.retention.unwrap_or_default(),
            "config_path": crate::config::path(),
        }))
    }

    fn save_settings(&self, s: SettingsIn) -> R<Value> {
        let restart_p2p = s.p2p_port.is_some();
        let r = self.edit_config(|c| {
            if let Some(n) = s.machine_name.filter(|n| !n.trim().is_empty()) {
                c.machine_name = n.trim().to_string();
            }
            if let Some(a) = s.auto_detect {
                c.auto_detect = a;
            }
            if let Some(p) = s.periodic_secs {
                c.periodic_secs = Some(p.max(60));
            }
            if let Some(k) = s.steamgriddb_key {
                c.steamgriddb_key = Some(k.trim().to_string()).filter(|k| !k.is_empty());
            }
            if let Some(p) = s.p2p_port {
                c.p2p_port = Some(p);
            }
            if let Some(r) = s.retention {
                c.retention = Some(r);
            }
            for (name, kbps) in &s.cloud_limits {
                if let Some(cl) = c.clouds.iter_mut().find(|x| &x.name == name) {
                    cl.limit_kbps = Some(*kbps).filter(|k| *k > 0);
                }
            }
            Ok(())
        })?;
        if restart_p2p {
            *self.p2p.lock().expect("lock") = None;
            if let Err(e) = self.start_p2p() {
                return Err(format!(
                    "ajustes salvos, mas a sincronização direta não subiu: {e}"
                ));
            }
        }
        Ok(r)
    }

    fn art(&self, game: &str, kind: ArtKind) -> R<Value> {
        let (cfg, games) = self.snapshot();
        let q = match games.iter().find(|g| g.id() == game) {
            Some(g) => g.art_query(),
            None => relicario_catalog::art::ArtQuery {
                id: game.to_string(),
                title: game.to_string(),
                ..Default::default()
            },
        };
        let info = crate::games::art_cache(&cfg)
            .get(&q, kind)
            .map_err(|e| e.to_string())?;
        let data = std::fs::read(&info.path).map_err(|e| e.to_string())?;
        Ok(json!({
            "url": data_url(mime_of(&info.path), &data),
            "source": info.source,
            "author": info.author,
        }))
    }

    fn export_zip(&self, game: &str, snapshot: &str, dest: &std::path::Path) -> R<Value> {
        use std::io::Write;
        let cfg = self.snapshot().0;
        let repo = cfg.open_repo()?;
        let sid = parse_id(snapshot)?;
        let s = repo.get_snapshot(&sid).map_err(|e| e.to_string())?;
        if s.game.as_str() != game {
            return Err(format!("snapshot {} não é de {game}", sid.short()));
        }
        let tree = repo.get_tree(&s.tree).map_err(|e| e.to_string())?;
        let file = std::fs::File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        let mut z = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for f in &tree.files {
            let data = relicario_store::files::read_entry(&repo, f).map_err(|e| e.to_string())?;
            z.start_file(format!("{}/{}", f.root, f.path), opts)
                .map_err(|e| e.to_string())?;
            z.write_all(&data).map_err(|e| e.to_string())?;
        }
        z.finish().map_err(|e| e.to_string())?;
        Ok(json!({"ok": true, "path": dest, "files": tree.files.len()}))
    }

    fn pair_join(&self, code: &str, addr: Option<&str>) -> R<Value> {
        let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
        if code.len() != 6 {
            return Err("o código tem 6 números".into());
        }
        let cfg = self.snapshot().0;
        let id = crate::p2p::identity()?;
        crate::p2p::publish_card(&cfg);
        let targets = crate::p2p::pairing_targets(addr, Duration::from_secs(3));
        if targets.is_empty() {
            return Err(
                "não achei nenhuma máquina esperando pareamento na rede; informe o endereço".into(),
            );
        }
        let peer = relicario_p2p::join(
            &id,
            &cfg.machine_id,
            &cfg.machine_name,
            &targets,
            &code,
            Duration::from_secs(5),
        )
        .map_err(|e| e.to_string())?;
        let name = peer.name.clone();
        crate::p2p::save_peer(peer)?;
        self.reload(false)?;
        crate::activity::record("pareamento", None, format!("pareado com {name}"));
        let (msgs, changed) = self.sync_peers(Duration::from_secs(5));
        if changed {
            self.remote_changed.store(true, Ordering::SeqCst);
        }
        Ok(json!({"ok": true, "name": name, "messages": msgs}))
    }
}

/// Faz o pedido ao daemon rodando ou, sem daemon, a um daemon temporário neste processo (sem o
/// laço e sem a sincronização direta).
pub fn call(c: Call) -> R<Value> {
    use crate::ipc::{self, Request, Response};
    match ipc::request(&Request::Api { call: c.clone() }) {
        Ok(Response::Api { value }) => Ok(value),
        Ok(Response::Error { message }) => Err(message),
        Ok(other) => Err(format!("resposta inesperada do daemon: {other:?}")),
        Err(_) => Daemon::new(Config::load()?)?.api(c),
    }
}
