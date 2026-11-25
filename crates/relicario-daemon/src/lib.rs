//! Serviço do Relicário: vigia os jogos, guarda o save quando o jogo fecha, traz o save de outra
//! máquina antes de abrir e responde à CLI e à interface por um socket local.

pub mod config;
pub mod games;
pub mod ipc;
pub mod lifecycle;
pub mod server;
pub mod state;
