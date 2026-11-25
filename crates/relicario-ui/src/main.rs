//! Interface do Relicário: janela com as telas do protótipo (`ui/`) e ícone na bandeja.
//!
//! A interface fala com o daemon pelo socket quando ele já está rodando (serviço instalado).
//! Sem daemon, sobe um dentro do próprio app; fechar a janela só esconde, e o app continua na
//! bandeja vigiando os jogos.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use relicario_daemon::config::Config;
use relicario_daemon::ipc::{self, Request, Response};
use relicario_daemon::server::Daemon;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;

/// Com quem a interface fala.
#[derive(Clone)]
enum Backend {
    /// Daemon dentro deste processo.
    Embedded(Arc<Daemon>),
    /// Daemon de fora (serviço), pelo socket.
    Remote,
}

#[derive(Default)]
struct App {
    backend: Mutex<Option<Backend>>,
}

impl App {
    /// Liga no daemon que estiver rodando ou sobe um aqui. `None` se ainda não configurado.
    fn connect(&self) -> Result<Option<Backend>, String> {
        let mut b = self.backend.lock().expect("lock");
        if let Some(x) = b.as_ref() {
            return Ok(Some(x.clone()));
        }
        if ipc::is_up() {
            *b = Some(Backend::Remote);
        } else if relicario_daemon::setup::configured() {
            let cfg = Config::load()?;
            *b = Some(match relicario_daemon::server::spawn(cfg) {
                Ok(d) => Backend::Embedded(d),
                // Outro subiu no meio do caminho.
                Err(_) if ipc::is_up() => Backend::Remote,
                Err(e) => return Err(e),
            });
        }
        Ok(b.clone())
    }

    fn request(&self, req: Request) -> Result<Response, String> {
        match self.connect()? {
            Some(Backend::Embedded(d)) => Ok(d.handle(req)),
            Some(Backend::Remote) => match ipc::request(&req) {
                Ok(r) => Ok(r),
                Err(e) => {
                    // O serviço caiu: na próxima, sobe um aqui.
                    *self.backend.lock().expect("lock") = None;
                    Err(format!("o serviço do Relicário parou de responder ({e})"))
                }
            },
            None => Err("o Relicário ainda não foi configurado nesta máquina".into()),
        }
    }

    fn api(&self, call: Value) -> Result<Value, String> {
        let call: relicario_daemon::api::Call =
            serde_json::from_value(call).map_err(|e| format!("pedido inválido: {e}"))?;
        match self.request(Request::Api { call })? {
            Response::Api { value } => Ok(value),
            Response::Error { message } => Err(message),
            other => Err(format!("resposta inesperada: {other:?}")),
        }
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

/// Estado do app para a primeira tela.
#[tauri::command]
async fn app_status(app: State<'_, Arc<App>>) -> Result<Value, String> {
    let app = app.inner().clone();
    blocking(move || {
        let backend = app.connect().ok().flatten();
        Ok(json!({
            "configured": relicario_daemon::setup::configured(),
            "backend": match backend {
                Some(Backend::Embedded(_)) => "embutido",
                Some(Backend::Remote) => "servico",
                None => "nenhum",
            },
            "default_repo": relicario_daemon::setup::default_repo(),
            "hostname": relicario_platform::hostname(),
            "version": env!("CARGO_PKG_VERSION"),
        }))
    })
    .await
}

/// Primeira execução: cria o repositório e liga o daemon.
#[tauri::command]
async fn setup(
    app: State<'_, Arc<App>>,
    name: String,
    repo: Option<String>,
) -> Result<Value, String> {
    let app = app.inner().clone();
    blocking(move || {
        let cfg =
            relicario_daemon::setup::init(&name, repo.filter(|r| !r.is_empty()).map(Into::into))?;
        // Um daemon já ligado continua com a configuração velha: recarrega.
        if let Some(b) = app.connect()? {
            let _ = match b {
                Backend::Embedded(d) => d.handle(Request::Reload),
                Backend::Remote => ipc::request(&Request::Reload).map_err(|e| e.to_string())?,
            };
        }
        Ok(json!({"machine": cfg.machine_name, "repo": cfg.repo}))
    })
    .await
}

/// Pedido da interface (veja `relicario_daemon::api::Call`).
#[tauri::command]
async fn api(app: State<'_, Arc<App>>, call: Value) -> Result<Value, String> {
    let app = app.inner().clone();
    blocking(move || app.api(call)).await
}

/// Pedido direto ao daemon (sincronizar, parear...).
#[tauri::command]
async fn daemon(app: State<'_, Arc<App>>, req: Value) -> Result<Value, String> {
    let app = app.inner().clone();
    blocking(move || {
        let req: Request =
            serde_json::from_value(req).map_err(|e| format!("pedido inválido: {e}"))?;
        match app.request(req)? {
            Response::Error { message } => Err(message),
            r => serde_json::to_value(r).map_err(|e| e.to_string()),
        }
    })
    .await
}

/// Código novo de pareamento (6 números).
#[tauri::command]
fn new_pair_code() -> String {
    relicario_daemon::p2p::new_code()
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |p| {
        let _ = tx.send(p.map(|p| p.to_string()));
    });
    blocking(move || rx.recv().map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn pick_save_path(app: AppHandle, name: String) -> Result<Option<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .set_file_name(&name)
        .add_filter("zip", &["zip"])
        .save_file(move |p| {
            let _ = tx.send(p.map(|p| p.to_string()));
        });
    blocking(move || rx.recv().map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn pick_image(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .add_filter("imagem", &["png", "jpg", "jpeg", "webp"])
        .pick_file(move |p| {
            let _ = tx.send(p.map(|p| p.to_string()));
        });
    blocking(move || rx.recv().map_err(|e| e.to_string())).await
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Bandeja: abrir, sincronizar, estado e sair. O texto do estado é atualizado a cada 15 s.
fn tray(app: &AppHandle, state: Arc<App>) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Relicário", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Abrir o Relicário", true, None::<&str>)?;
    let sync = MenuItem::with_id(app, "sync", "Sincronizar agora", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &sync,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-44.png"))?;
    let st = state.clone();
    let tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("Relicário")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, ev| match ev.id().as_ref() {
            "open" => show_main(app),
            "sync" => {
                let st = st.clone();
                std::thread::spawn(move || {
                    let _ = st.request(Request::Sync { game: None });
                });
            }
            "quit" => {
                if let Some(Backend::Embedded(d)) = st.backend.lock().expect("lock").clone() {
                    d.stop();
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|t, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                show_main(t.app_handle());
            }
        })
        .build(app)?;
    std::thread::spawn(move || {
        loop {
            let text = match state.api(json!({"call": "overview"})) {
                Ok(v) => {
                    let conflicts = v["conflicts"].as_u64().unwrap_or(0);
                    let running: Vec<&str> = v["running"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                        .unwrap_or_default();
                    if conflicts > 0 {
                        format!("{conflicts} conflito(s) para resolver")
                    } else if v["cloud_busy"] == true {
                        "Enviando para a nuvem…".to_string()
                    } else if !running.is_empty() {
                        format!("Jogando: {}", running.join(", "))
                    } else {
                        "Tudo em dia".to_string()
                    }
                }
                Err(_) => "Relicário (não configurado)".to_string(),
            };
            let _ = status.set_text(&text);
            let _ = tray.set_tooltip(Some(format!("Relicário · {text}")));
            std::thread::sleep(Duration::from_secs(15));
        }
    });
    Ok(())
}

/// O WebKitGTK com o renderizador DMA-BUF desenha só parte da janela nos drivers da NVIDIA
/// (o conteúdo fica cortado no tamanho em que a janela abriu e pedaços aparecem soltos). A
/// variável que desliga esse renderizador precisa existir antes de o WebKit carregar, e mudar o
/// ambiente do próprio processo exige `unsafe`; então o app se reabre uma vez com ela.
/// `RELICARIO_WEBKIT_DMABUF=1` mantém o renderizador padrão.
#[cfg(target_os = "linux")]
fn reabrir_sem_dmabuf() {
    const MARCA: &str = "RELICARIO_WEBKIT_REABERTO";
    let nvidia = std::path::Path::new("/proc/driver/nvidia/version").exists();
    if !nvidia
        || std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some()
        || std::env::var_os("RELICARIO_WEBKIT_DMABUF").is_some()
        || std::env::var_os(MARCA).is_some()
    {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let status = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
        .env(MARCA, "1")
        .status();
    if let Ok(status) = status {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    reabrir_sem_dmabuf();
    // No modo Jogo do Steam Deck a tela é do Steam: o app fica só na bandeja, sem janela.
    let hidden = std::env::args().any(|a| a == "--hidden" || a == "--bandeja")
        || relicario_platform::deck::detect().game_mode;
    let state = Arc::new(App::default());
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            app_status,
            setup,
            api,
            daemon,
            new_pair_code,
            pick_folder,
            pick_save_path,
            pick_image
        ])
        .setup(move |app| {
            tray(app.handle(), state.clone())?;
            if !hidden {
                show_main(app.handle());
            }
            Ok(())
        })
        .on_window_event(|w, ev| {
            // Fechar a janela só esconde: o Relicário continua na bandeja.
            if let WindowEvent::CloseRequested { api, .. } = ev {
                api.prevent_close();
                let _ = w.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("erro ao iniciar a interface");
}
