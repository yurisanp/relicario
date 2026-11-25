//! Serviço do Relicário: vigia os jogos, guarda o save quando o jogo fecha, traz o save de outra
//! máquina antes de abrir e responde à CLI e à interface por um socket local.

pub mod activity;
pub mod api;
pub mod cloud;
pub mod config;
pub mod games;
pub mod ipc;
pub mod lifecycle;
pub mod ludusavi;
pub mod oauth;
pub mod p2p;
pub mod server;
pub mod setup;
pub mod state;
