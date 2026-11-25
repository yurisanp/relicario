//! Esperar o jogo terminar de gravar: nenhuma mudança por alguns segundos e, no Windows, nenhum
//! arquivo travado por outro processo.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Impressão barata do estado de uma lista de arquivos (tamanho e horário de cada um).
pub fn fingerprint(files: &[PathBuf]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for f in files {
        let (len, mtime) = std::fs::metadata(f)
            .map(|m| {
                let t = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as u64);
                (m.len(), t.unwrap_or(0))
            })
            .unwrap_or((u64::MAX, 0));
        for x in [len, mtime, f.as_os_str().len() as u64] {
            h ^= x;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// Algum arquivo está aberto com trava exclusiva por outro processo? Só faz sentido no Windows;
/// nos outros sistemas devolve `false`.
pub fn any_locked(files: &[PathBuf]) -> bool {
    files.iter().any(|f| is_locked(f))
}

#[cfg(windows)]
fn is_locked(p: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    match std::fs::OpenOptions::new().read(true).share_mode(0).open(p) {
        Ok(_) => false,
        Err(e) => e.raw_os_error() == Some(32), // ERROR_SHARING_VIOLATION
    }
}

#[cfg(not(windows))]
fn is_locked(_p: &Path) -> bool {
    false
}

/// Espera até `list()` ficar estável por `quiet`, ou até `timeout`. Devolve `true` se estabilizou.
pub fn wait_settled(
    mut list: impl FnMut() -> Vec<PathBuf>,
    quiet: Duration,
    timeout: Duration,
    step: Duration,
) -> bool {
    let start = Instant::now();
    let mut last = None;
    let mut stable_since = Instant::now();
    loop {
        let files = list();
        let fp = fingerprint(&files);
        if Some(fp) != last || any_locked(&files) {
            last = Some(fp);
            stable_since = Instant::now();
        } else if stable_since.elapsed() >= quiet {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(step);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn espera_gravacao_aos_poucos() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("save.dat");
        let f2 = f.clone();
        let writer = std::thread::spawn(move || {
            for i in 0..5 {
                std::fs::write(&f2, vec![i as u8; 100 * (i + 1)]).unwrap();
                std::thread::sleep(Duration::from_millis(60));
            }
        });
        let start = Instant::now();
        let ok = wait_settled(
            || vec![f.clone()],
            Duration::from_millis(200),
            Duration::from_secs(5),
            Duration::from_millis(20),
        );
        writer.join().unwrap();
        assert!(ok);
        assert!(
            start.elapsed() >= Duration::from_millis(400),
            "esperou a escrita acabar"
        );
        assert_eq!(std::fs::read(&f).unwrap().len(), 500);
    }

    #[test]
    fn desiste_no_tempo_limite() {
        let mut n = 0u32;
        let d = tempfile::tempdir().unwrap();
        let ok = wait_settled(
            || {
                n += 1;
                let p = d.path().join(format!("{n}"));
                std::fs::write(&p, b"x").unwrap();
                vec![p]
            },
            Duration::from_millis(100),
            Duration::from_millis(300),
            Duration::from_millis(10),
        );
        assert!(!ok);
    }
}
