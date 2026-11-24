//! `relicario`: linha de comando.

use std::process::ExitCode;

const AJUDA: &str = "\
relicario: mantém os saves dos seus jogos iguais em todas as máquinas

Uso: relicario <comando>

Comandos:
  doctor     Mostra informações do ambiente
  version    Mostra a versão
  help       Mostra esta ajuda

Os comandos init, snapshot, plan, restore, log, pin e verify chegam na fase 1.";

fn main() -> ExitCode {
    let comando = std::env::args().nth(1);
    match comando.as_deref() {
        None | Some("help" | "-h" | "--help") => println!("{AJUDA}"),
        Some("version" | "-V" | "--version") => println!("relicario {}", env!("CARGO_PKG_VERSION")),
        Some("doctor") => {
            println!("relicario {}", env!("CARGO_PKG_VERSION"));
            println!("sistema: {}", relicario_platform::Os::atual().nome());
        }
        Some(outro) => {
            eprintln!("comando desconhecido: {outro}\n\n{AJUDA}");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}
