//! `relicario-daemon`: serviço do usuário que vigia os jogos e sincroniza os saves.

fn main() {
    if let Err(e) = relicario_daemon::server::run() {
        eprintln!("relicario-daemon: {e}");
        std::process::exit(1);
    }
}
