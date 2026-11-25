//! Notificações nativas, sem bloquear quem chama. Falhas são ignoradas: notificação é aviso,
//! nunca condição para seguir.

use std::process::{Command, Stdio};

/// Mostra uma notificação. `RELICARIO_NO_NOTIFY=1` desliga (testes, servidores).
pub fn notify(title: &str, body: &str) {
    if std::env::var_os("RELICARIO_NO_NOTIFY").is_some() {
        return;
    }
    let mut cmd = command(title, body);
    if let Some(c) = cmd.as_mut() {
        let _ = c
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

fn command(title: &str, body: &str) -> Option<Command> {
    if cfg!(target_os = "macos") {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            body.replace('"', "'"),
            title.replace('"', "'")
        );
        let mut c = Command::new("osascript");
        c.arg("-e").arg(script);
        Some(c)
    } else if cfg!(windows) {
        let esc = |s: &str| {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('\'', "&apos;")
        };
        let ps = format!(
            "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
             $x = New-Object Windows.Data.Xml.Dom.XmlDocument; \
             $x.LoadXml('<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>'); \
             [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Relicario').Show([Windows.UI.Notifications.ToastNotification]::new($x))",
            esc(title),
            esc(body)
        );
        let mut c = Command::new("powershell");
        c.args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &ps,
        ]);
        Some(c)
    } else {
        let mut c = Command::new("notify-send");
        c.args(["--app-name=Relicário", "--icon=relicario", title, body]);
        Some(c)
    }
}
