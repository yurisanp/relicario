//! Lista de processos do usuário.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Um processo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    pub exe: Option<PathBuf>,
    pub cmdline: Vec<String>,
}

/// Processos em execução.
#[cfg(target_os = "linux")]
pub fn list() -> Vec<ProcInfo> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return out;
    };
    for e in rd.filter_map(|e| e.ok()) {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let base = e.path();
        let Ok(name) = std::fs::read_to_string(base.join("comm")) else {
            continue;
        };
        let exe = std::fs::read_link(base.join("exe")).ok();
        let cmdline = std::fs::read(base.join("cmdline"))
            .map(|b| {
                b.split(|c| *c == 0)
                    .filter(|s| !s.is_empty())
                    .map(|s| String::from_utf8_lossy(s).into_owned())
                    .collect()
            })
            .unwrap_or_default();
        out.push(ProcInfo {
            pid,
            name: name.trim().to_string(),
            exe,
            cmdline,
        });
    }
    out
}

/// Processos em execução.
#[cfg(not(target_os = "linux"))]
pub fn list() -> Vec<ProcInfo> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut s = System::new();
    s.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );
    s.processes()
        .iter()
        .map(|(pid, p)| ProcInfo {
            pid: pid.as_u32(),
            name: p.name().to_string_lossy().into_owned(),
            exe: p.exe().map(|e| e.to_path_buf()),
            cmdline: p
                .cmd()
                .iter()
                .map(|c| c.to_string_lossy().into_owned())
                .collect(),
        })
        .collect()
}
