//! Steam Deck (e outras máquinas com SteamOS). No modo Jogo a tela é do Steam (gamescope):
//! o Relicário fica só na bandeja/serviço, sem abrir janela.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Deck {
    /// SteamOS ou hardware do Steam Deck.
    pub steamos: bool,
    /// Rodando dentro do modo Jogo (gamescope).
    pub game_mode: bool,
}

/// Detecta pelo sistema e pelo ambiente. Em Windows e macOS, sempre falso.
pub fn detect() -> Deck {
    if !cfg!(target_os = "linux") {
        return Deck::default();
    }
    let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let board = read_trim("/sys/devices/virtual/dmi/id/board_vendor");
    let product = read_trim("/sys/devices/virtual/dmi/id/product_name");
    from_parts(&os_release, &board, &product, |k| std::env::var(k).ok())
}

fn read_trim(p: impl AsRef<Path>) -> String {
    std::fs::read_to_string(p)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn from_parts(
    os_release: &str,
    board_vendor: &str,
    product: &str,
    env: impl Fn(&str) -> Option<String>,
) -> Deck {
    let steamos = os_release
        .lines()
        .any(|l| l.trim() == "ID=steamos" || l.trim() == "ID=\"steamos\"")
        || (board_vendor == "Valve" && matches!(product, "Jupiter" | "Galileo"));
    let game_mode = env("XDG_CURRENT_DESKTOP").is_some_and(|d| d.eq_ignore_ascii_case("gamescope"))
        || env("GAMESCOPE_WAYLAND_DISPLAY").is_some()
        || env("SteamGamepadUI").is_some_and(|v| v == "1");
    Deck { steamos, game_mode }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deck_no_modo_jogo_e_no_desktop() {
        let none = |_: &str| None;
        let d = from_parts("ID=steamos\nVERSION_ID=3.6", "", "", none);
        assert_eq!(
            d,
            Deck {
                steamos: true,
                game_mode: false
            }
        );
        let d = from_parts("ID=arch", "Valve", "Galileo", |k| {
            (k == "XDG_CURRENT_DESKTOP").then(|| "gamescope".to_string())
        });
        assert_eq!(
            d,
            Deck {
                steamos: true,
                game_mode: true
            }
        );
        let d = from_parts("ID=fedora", "LENOVO", "x", none);
        assert_eq!(d, Deck::default());
    }
}
