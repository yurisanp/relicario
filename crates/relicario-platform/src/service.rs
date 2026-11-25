//! Daemon como serviço do usuário, sem precisar de administrador.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Nome do serviço.
pub const NAME: &str = "relicario";
/// Rótulo do LaunchAgent no macOS.
pub const MAC_LABEL: &str = "io.github.yurisanp.relicario";

/// Executável a registrar: dentro de um AppImage, o próprio AppImage (o caminho montado muda a
/// cada execução).
pub fn current_exe() -> std::io::Result<PathBuf> {
    if let Some(a) = std::env::var_os("APPIMAGE") {
        return Ok(PathBuf::from(a));
    }
    std::env::current_exe()
}

/// Unidade systemd `--user`.
pub fn systemd_unit(exe: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=Relicário: mantém os saves dos jogos iguais entre máquinas\n\
         After=graphical-session.target network-online.target\n\
         \n\
         [Service]\n\
         ExecStart=\"{}\" daemon run\n\
         Restart=on-failure\n\
         RestartSec=10\n\
         Nice=10\n\
         IOSchedulingClass=idle\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe.display()
    )
}

/// LaunchAgent do macOS.
pub fn launch_agent(exe: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{MAC_LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{}</string><string>daemon</string><string>run</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key><string>Background</string>
  <key>LowPriorityIO</key><true/>
</dict>
</plist>
"#,
        exe.display()
    )
}

fn home() -> PathBuf {
    crate::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn systemd_unit_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("systemd/user/relicario.service")
}

pub fn launch_agent_path() -> PathBuf {
    home()
        .join("Library/LaunchAgents")
        .join(format!("{MAC_LABEL}.plist"))
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{cmd:?}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Instala e inicia o serviço. Devolve uma descrição do que foi feito.
pub fn install(exe: &Path) -> Result<String, String> {
    match crate::Os::atual() {
        crate::Os::Windows => {
            let tr = format!("\"{}\" daemon run", exe.display());
            run(Command::new("schtasks").args([
                "/Create",
                "/TN",
                "Relicario",
                "/TR",
                &tr,
                "/SC",
                "ONLOGON",
                "/RL",
                "LIMITED",
                "/F",
            ]))?;
            let _ = run(Command::new("schtasks").args(["/Run", "/TN", "Relicario"]));
            Ok("tarefa agendada \"Relicario\" criada (roda ao entrar no Windows)".into())
        }
        crate::Os::MacOs => {
            let p = launch_agent_path();
            write(&p, &launch_agent(exe))?;
            let _ = run(Command::new("launchctl").args(["unload", &p.to_string_lossy()]));
            run(Command::new("launchctl").args(["load", "-w", &p.to_string_lossy()]))?;
            Ok(format!("LaunchAgent instalado em {}", p.display()))
        }
        _ => {
            let p = systemd_unit_path();
            write(&p, &systemd_unit(exe))?;
            run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
            run(Command::new("systemctl").args([
                "--user",
                "enable",
                "--now",
                "relicario.service",
            ]))?;
            Ok(format!(
                "serviço systemd --user instalado em {}",
                p.display()
            ))
        }
    }
}

/// Para e remove o serviço.
pub fn uninstall() -> Result<String, String> {
    match crate::Os::atual() {
        crate::Os::Windows => {
            run(Command::new("schtasks").args(["/Delete", "/TN", "Relicario", "/F"]))?;
            Ok("tarefa agendada removida".into())
        }
        crate::Os::MacOs => {
            let p = launch_agent_path();
            let _ = run(Command::new("launchctl").args(["unload", "-w", &p.to_string_lossy()]));
            let _ = std::fs::remove_file(&p);
            Ok("LaunchAgent removido".into())
        }
        _ => {
            let _ = run(Command::new("systemctl").args([
                "--user",
                "disable",
                "--now",
                "relicario.service",
            ]));
            let _ = std::fs::remove_file(systemd_unit_path());
            let _ = run(Command::new("systemctl").args(["--user", "daemon-reload"]));
            Ok("serviço systemd --user removido".into())
        }
    }
}

fn write(p: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(p, text).map_err(|e| format!("{}: {e}", p.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arquivos_de_servico() {
        let u = systemd_unit(Path::new("/opt/Relicario.AppImage"));
        assert!(u.contains("ExecStart=\"/opt/Relicario.AppImage\" daemon run"));
        assert!(u.contains("WantedBy=default.target"));
        let l = launch_agent(Path::new(
            "/Applications/Relicario.app/Contents/MacOS/relicario",
        ));
        assert!(l.contains(MAC_LABEL) && l.contains("<string>daemon</string>"));
    }
}
