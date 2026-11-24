//! `relicario-daemon`: serviço do usuário que vigia os jogos e sincroniza os saves.
//!
//! Fase 0: só se apresenta. A orquestração (filas, travas por jogo, SQLite e API JSON-RPC)
//! chega na fase 3.

fn main() {
    println!(
        "relicario-daemon {} ({}): ainda não faz nada; veja docs/roadmap.md",
        env!("CARGO_PKG_VERSION"),
        relicario_platform::Os::atual().nome()
    );
}
