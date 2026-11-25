//! Limite de banda para um destino.

use crate::{Kind, Remote, Result};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Embrulha um destino e espera o suficiente para não passar de `bytes_per_sec` no envio e no
/// recebimento.
#[derive(Debug)]
pub struct Limited<R> {
    inner: R,
    bytes_per_sec: u64,
    next: Mutex<Instant>,
}

impl<R: Remote> Limited<R> {
    pub fn new(inner: R, bytes_per_sec: u64) -> Self {
        Limited {
            inner,
            bytes_per_sec: bytes_per_sec.max(1024),
            next: Mutex::new(Instant::now()),
        }
    }

    fn pace(&self, n: usize) {
        let cost = Duration::from_secs_f64(n as f64 / self.bytes_per_sec as f64);
        let wait = {
            let mut next = self.next.lock().expect("lock");
            let now = Instant::now();
            let start = (*next).max(now);
            *next = start + cost;
            start.saturating_duration_since(now)
        };
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }
}

impl<R: Remote> Remote for Limited<R> {
    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> Result<()> {
        self.pace(data.len());
        self.inner.put(kind, name, data)
    }
    fn get(&self, kind: Kind, name: &str) -> Result<Option<Vec<u8>>> {
        let r = self.inner.get(kind, name)?;
        if let Some(d) = &r {
            self.pace(d.len());
        }
        Ok(r)
    }
    fn list(&self, kind: Kind) -> Result<Vec<String>> {
        self.inner.list(kind)
    }
    fn delete(&self, kind: Kind, name: &str) -> Result<()> {
        self.inner.delete(kind, name)
    }
    fn describe(&self) -> String {
        format!(
            "{} (até {} KiB/s)",
            self.inner.describe(),
            self.bytes_per_sec / 1024
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LocalFolder;

    #[test]
    fn respeita_o_limite() {
        let t = tempfile::tempdir().unwrap();
        let l = Limited::new(LocalFolder::new(t.path()), 200 * 1024);
        let start = Instant::now();
        for i in 0..4 {
            l.put(Kind::Packs, &format!("{i:02}aa"), &vec![0u8; 50 * 1024])
                .unwrap();
        }
        // 200 KiB a 200 KiB/s: pelo menos ~0,75 s (o primeiro envio não espera).
        assert!(
            start.elapsed() >= Duration::from_millis(700),
            "{:?}",
            start.elapsed()
        );
    }
}
