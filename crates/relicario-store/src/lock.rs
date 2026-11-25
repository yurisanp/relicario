use crate::error::{Error, Result, json};
use relicario_model::MachineId;
use relicario_transport::{Kind, Remote};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Validade de uma trava. Travas vencidas são ignoradas (processo que morreu).
pub const LOCK_TTL_MS: i64 = 30 * 60 * 1000;

/// Tipo de trava.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockMode {
    /// Escrita normal (snapshot). Várias máquinas ao mesmo tempo.
    Shared,
    /// Prune/GC: ninguém mais pode estar escrevendo.
    Exclusive,
}

#[derive(Debug, Serialize, Deserialize)]
struct LockFile {
    machine: MachineId,
    mode: LockMode,
    expires_at: i64,
}

/// Trava ativa; é removida ao sair do escopo.
#[derive(Debug)]
pub struct LockGuard {
    remote: Arc<dyn Remote>,
    name: String,
}

impl LockGuard {
    pub(crate) fn acquire(
        remote: Arc<dyn Remote>,
        machine: &MachineId,
        mode: LockMode,
    ) -> Result<Self> {
        let now = crate::time::now();
        let name = format!(
            "{}-{}",
            machine.as_str(),
            match mode {
                LockMode::Shared => "shared",
                LockMode::Exclusive => "exclusive",
            }
        );
        let conflicts = |remote: &Arc<dyn Remote>| -> Result<Option<String>> {
            for n in remote.list(Kind::Locks)? {
                if n == name {
                    continue;
                }
                let Some(bytes) = remote.get(Kind::Locks, &n)? else {
                    continue;
                };
                let Ok(l) = serde_json::from_slice::<LockFile>(&bytes) else {
                    continue;
                };
                if l.expires_at < now {
                    continue;
                }
                if mode == LockMode::Exclusive || l.mode == LockMode::Exclusive {
                    return Ok(Some(format!("{} ({:?})", l.machine, l.mode)));
                }
            }
            Ok(None)
        };
        if let Some(who) = conflicts(&remote)? {
            return Err(Error::Locked(who));
        }
        let lf = LockFile {
            machine: machine.clone(),
            mode,
            expires_at: now + LOCK_TTL_MS,
        };
        remote.put(
            Kind::Locks,
            &name,
            &serde_json::to_vec(&lf).map_err(json("lock"))?,
        )?;
        // Confere de novo depois de escrever, para pegar quem travou ao mesmo tempo.
        if let Some(who) = conflicts(&remote)? {
            let _ = remote.delete(Kind::Locks, &name);
            return Err(Error::Locked(who));
        }
        Ok(LockGuard { remote, name })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.remote.delete(Kind::Locks, &self.name);
    }
}
