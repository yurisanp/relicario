//! Horário em milissegundos Unix e formatação simples em UTC, sem dependências.

use relicario_model::Millis;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> Millis {
    to_millis(SystemTime::now())
}

pub fn to_millis(t: SystemTime) -> Millis {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as Millis,
        Err(e) => -(e.duration().as_millis() as Millis),
    }
}

pub fn from_millis(ms: Millis) -> SystemTime {
    if ms >= 0 {
        UNIX_EPOCH + std::time::Duration::from_millis(ms as u64)
    } else {
        UNIX_EPOCH - std::time::Duration::from_millis(ms.unsigned_abs())
    }
}

/// `AAAA-MM-DD HH:MM:SS` em UTC.
pub fn format_utc(ms: Millis) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

// Algoritmo de Howard Hinnant.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    #[test]
    fn formata() {
        assert_eq!(super::format_utc(0), "1970-01-01 00:00:00");
        assert_eq!(super::format_utc(1_795_996_800_000), "2026-11-30 00:00:00");
    }
}
