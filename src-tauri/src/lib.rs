// Ecxon Connect — biblioteca principal.
// Inicializa Tauri, plugins, system tray, single-instance e o servidor HTTP local.

mod http_server;
mod probe;
mod tray;

use std::sync::{Arc, Mutex};
use serde::Serialize;
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::MacosLauncher;

/// Estado compartilhado do agente.
#[derive(Default, Clone, Serialize)]
pub struct ServerStatus {
    pub running: bool,
    pub bind_addr: String,
    pub error: Option<String>,
}

pub struct AppState {
    pub server_status: Mutex<ServerStatus>,
}

#[tauri::command]
fn get_server_status(state: tauri::State<'_, Arc<AppState>>) -> ServerStatus {
    state.server_status.lock().unwrap().clone()
}

#[tauri::command]
fn open_main_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        Ok(())
    } else {
        Err("janela principal não encontrada".into())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Logger básico — útil em dev e em release (sem console em release/Windows).
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    // Em release Windows não há console — registra panics num arquivo
    // (%TEMP%\ecxon-connect-panic.log) pra debugar crashes silenciosos.
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<sem localização>".to_string());
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<payload de panic não-string>".to_string()
        };
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = format!(
            "===== ecxon-connect PANIC =====\n\
             timestamp: {timestamp}\n\
             location:  {location}\n\
             message:   {msg}\n\
             ================================\n\n"
        );
        let path = std::env::temp_dir().join("ecxon-connect-panic.log");
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map(|mut f| {
                use std::io::Write;
                let _ = f.write_all(entry.as_bytes());
            });
        eprintln!("{entry}");
    }));

    let state = Arc::new(AppState {
        server_status: Mutex::new(ServerStatus {
            running: false,
            bind_addr: http_server::BIND_ADDR.to_string(),
            error: None,
        }),
    });

    let mut builder = tauri::Builder::default();

    // Single-instance — desktop only.
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // Segunda instância -> traz a janela existente pra frente.
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![get_server_status, open_main_window])
        .setup({
            let state = state.clone();
            move |app| {
                // Registra o tray.
                if let Err(e) = tray::setup(app.handle()) {
                    log::error!("falha ao criar tray: {e}");
                }

                // Sobe o servidor HTTP em uma task tokio dedicada.
                let app_handle = app.handle().clone();
                let state_for_server = state.clone();
                tauri::async_runtime::spawn(async move {
                    let on_ready_handle = app_handle.clone();
                    let on_ready_state = state_for_server.clone();
                    let on_ready: http_server::OnReadyCb = Box::new(move || {
                        // Bind 5556 estabelecido — atualiza status pra "running".
                        // axum::serve não retorna em condições normais, então
                        // marcamos running aqui (e não no Ok(()) abaixo).
                        let mut s = on_ready_state.server_status.lock().unwrap();
                        s.running = true;
                        s.error = None;
                        let _ = on_ready_handle.emit("server-status", s.clone());
                        log::info!("servidor HTTP pronto em {}", http_server::BIND_ADDR);
                    });
                    match http_server::start(app_handle.clone(), on_ready).await {
                        Ok(()) => {
                            log::info!("servidor HTTP encerrado normalmente");
                        }
                        Err(err) => {
                            let msg = format!("{err}");
                            log::error!("servidor HTTP falhou: {msg}");
                            {
                                let mut s = state_for_server.server_status.lock().unwrap();
                                s.running = false;
                                s.error = Some(friendly_bind_error(&msg));
                                let _ = app_handle.emit("server-status", s.clone());
                            }
                            // Mostra erro também no tooltip do tray.
                            if let Some(tray) = app_handle.tray_by_id("main-tray") {
                                let _ = tray.set_tooltip(Some(format!(
                                    "Ecxon Connect — ERRO: {}",
                                    friendly_bind_error(&msg)
                                )));
                            }
                            // Imprime no stderr (visível em dev / quando rodando do terminal).
                            eprintln!(
                                "[ecxon-connect] ERRO ao subir servidor: {}",
                                friendly_bind_error(&msg)
                            );
                        }
                    }
                });

                Ok(())
            }
        })
        .on_window_event(|window, event| {
            // Fechar janela = só esconde (mantém na tray).
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("erro ao iniciar Ecxon Connect");
}

fn friendly_bind_error(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("address already in use")
        || lower.contains("only one usage of each socket")
        || lower.contains("eaddrinuse")
    {
        "Porta 5556 já em uso — feche o agente antigo (ecxon-diag-agent) ou outro Ecxon Connect.".to_string()
    } else if lower.contains("permission denied") || lower.contains("access is denied") {
        "Sem permissão para abrir a porta 5556 — verifique firewall/antivírus.".to_string()
    } else {
        format!("Falha ao iniciar servidor: {raw}")
    }
}
