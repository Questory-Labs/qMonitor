mod auth;
mod config;
mod db;
mod detect;
mod device;
mod health;
mod identity;
mod live_session;
mod oauth_loopback;
mod persist;
mod pkce;
mod push;
mod runtime;
mod session;
mod update_check;

use std::sync::Arc;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State, WindowEvent,
};
use tauri_plugin_autostart::MacosLauncher;
use tokio::sync::Mutex;

use auth::{AuthState, LoginAttempt};
use config::AppConfig;
use identity::{PendingDetection, TrackableGame};
use oauth_loopback::LoopbackGuard;
use session::{list_trackable_games, AppState, HomeState};
use update_check::PendingUpdate;

struct LoginListener(Arc<Mutex<Option<LoopbackGuard>>>);
struct LoginAttemptState(Arc<Mutex<Option<LoginAttempt>>>);

#[tauri::command]
async fn get_config(state: State<'_, Arc<AppState>>) -> Result<AppConfig, String> {
    Ok(state.config.read().await.clone())
}

#[tauri::command]
async fn save_config(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    mut config: AppConfig,
) -> Result<AppConfig, String> {
    let (prev_url, prev_channel) = {
        let current = state.config.read().await;
        (
            current.resolved_detectable_url(),
            current.update_channel,
        )
    };
    if config.base_url.as_ref().is_some_and(|u| !u.trim().is_empty()) {
        auth::detect_and_apply(&mut config).await?;
    } else {
        config.api_root = None;
        config.web_origin = None;
        config.service = None;
    }
    let next_url = config.resolved_detectable_url();
    let url_changed = prev_url != next_url;
    let channel_changed = prev_channel != config.update_channel;
    let next_channel = config.update_channel;
    config.save()?;
    crate::health::apply_log_level(config.log_level);
    *state.config.write().await = config.clone();
    state.reload_pipeline().await;
    if url_changed {
        state.refresh_detectable(true).await;
    }
    // Local session DB is independent of Questory URL — always ensure it's open.
    if let Err(e) = state.connect_db().await {
        tracing::error!(%e, "failed to open local database after save_config");
    }
    if channel_changed {
        tauri::async_runtime::spawn(async move {
            match update_check::check(next_channel, true).await {
                Ok(pending) => emit_update_event(&app, pending.as_ref()),
                Err(e) => tracing::warn!(%e, "update check after channel change failed"),
            }
        });
    }
    Ok(config)
}

#[tauri::command]
async fn test_base_url(base_url: String) -> Result<String, String> {
    let mut cfg = AppConfig {
        base_url: Some(base_url),
        ..AppConfig::default()
    };
    let service = auth::detect_and_apply(&mut cfg).await?;
    Ok(format!("OK (service={service})"))
}

#[tauri::command]
async fn get_auth_state(state: State<'_, Arc<AppState>>) -> Result<Option<AuthState>, String> {
    let cfg = state.config.read().await.clone();
    Ok(auth::auth_state(&cfg))
}

#[tauri::command]
async fn start_login(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    listener: State<'_, LoginListener>,
    attempt: State<'_, LoginAttemptState>,
) -> Result<String, String> {
    let cfg = state.config.read().await.clone();
    if !cfg.has_base_url() {
        return Err("Set and verify a Questory URL first".into());
    }

    oauth_loopback::stop_listener(&listener.0);
    let (login_attempt, challenge, device_id) = auth::begin_login_attempt()?;
    let url = cfg
        .authorize_url(
            oauth_loopback::REDIRECT_URI,
            &login_attempt.state,
            &challenge,
            &device_id,
        )
        .ok_or_else(|| "web origin missing".to_string())?;

    *attempt.0.lock().await = Some(login_attempt);

    let app_state = Arc::clone(&*state);
    let guard =
        oauth_loopback::start_listener(app.clone(), attempt.0.clone(), app_state).await?;
    *listener.0.lock().await = Some(guard);

    open::that(&url).map_err(|e| e.to_string())?;
    let _ = app.emit("qmonitor://auth-waiting", ());
    Ok(url)
}

#[tauri::command]
async fn cancel_login(
    listener: State<'_, LoginListener>,
    attempt: State<'_, LoginAttemptState>,
) -> Result<(), String> {
    oauth_loopback::stop_listener(&listener.0);
    *attempt.0.lock().await = None;
    Ok(())
}

#[tauri::command]
async fn complete_login(
    state: State<'_, Arc<AppState>>,
    listener: State<'_, LoginListener>,
    attempt: State<'_, LoginAttemptState>,
    callback_url: String,
) -> Result<AuthState, String> {
    let (code, oauth_state) = auth::parse_callback_code(&callback_url)?;
    let login_attempt = attempt
        .0
        .lock()
        .await
        .clone()
        .ok_or_else(|| "no login attempt — click Log in first".to_string())?;
    let cfg = state.config.read().await.clone();
    auth::exchange_authorization_code(&cfg, &login_attempt, &code, &oauth_state).await?;
    *attempt.0.lock().await = None;
    oauth_loopback::stop_listener(&listener.0);
    auth::auth_state(&cfg).ok_or_else(|| "base URL missing".into())
}

#[tauri::command]
async fn set_dev_token(state: State<'_, Arc<AppState>>, token: String) -> Result<(), String> {
    let mut cfg = state.config.read().await.clone();
    cfg.dev_access_token = if token.is_empty() { None } else { Some(token) };
    cfg.save()?;
    *state.config.write().await = cfg;
    Ok(())
}

#[tauri::command]
async fn sign_out(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let cfg = state.config.read().await.clone();
    let _ = auth::revoke_remote(&cfg).await;
    auth::clear_tokens()?;
    let mut cfg = state.config.read().await.clone();
    cfg.dev_access_token = None;
    cfg.save()?;
    *state.config.write().await = cfg;
    Ok(())
}

#[tauri::command]
async fn get_home(state: State<'_, Arc<AppState>>) -> Result<HomeState, String> {
    Ok(state.home_state().await)
}

#[tauri::command]
async fn list_games(state: State<'_, Arc<AppState>>) -> Result<Vec<TrackableGame>, String> {
    Ok(list_trackable_games(&state).await)
}

#[tauri::command]
async fn list_pending(state: State<'_, Arc<AppState>>) -> Result<Vec<PendingDetection>, String> {
    Ok(state.pending_detections.read().await.clone())
}

#[tauri::command]
async fn confirm_game(
    state: State<'_, Arc<AppState>>,
    fingerprint: String,
    title: String,
) -> Result<(), String> {
    state.confirm_detection(fingerprint, title).await
}

#[tauri::command]
async fn ignore_game(
    state: State<'_, Arc<AppState>>,
    identity_id: String,
    title: String,
) -> Result<(), String> {
    state.ignore_game(identity_id, title).await
}

#[tauri::command]
async fn unignore_game(
    state: State<'_, Arc<AppState>>,
    identity_id: String,
) -> Result<(), String> {
    state.unignore_game(identity_id).await
}

#[tauri::command]
async fn add_manual_game(
    state: State<'_, Arc<AppState>>,
    title: String,
    exe_path: String,
    steam_app_id: Option<u32>,
) -> Result<(), String> {
    state
        .add_manual_game(title, exe_path, steam_app_id)
        .await
        .map(|_| ())
}

#[tauri::command]
async fn open_db(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    let path = state.config.read().await.resolved_db_path();
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "invalid database path".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // Open the folder, not the .db file — there is usually no default app.
    open::that(dir).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
fn open_log_dir() -> Result<String, String> {
    let dir = crate::health::log_dir();
    let _ = std::fs::create_dir_all(&dir);
    open::that(&dir).map_err(|e| e.to_string())?;
    Ok(dir.display().to_string())
}

#[tauri::command]
async fn is_onboarded(state: State<'_, Arc<AppState>>) -> Result<bool, String> {
    let cfg = state.config.read().await.clone();
    Ok(cfg.has_base_url() && auth::get_access_token(&cfg).is_some())
}

#[tauri::command]
fn login_listening() -> bool {
    oauth_loopback::is_listening()
}

#[tauri::command]
fn get_app_version() -> String {
    update_check::installed_version().to_string()
}

#[tauri::command]
async fn check_for_updates(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    force: bool,
) -> Result<Option<PendingUpdate>, String> {
    let channel = state.config.read().await.update_channel;
    let pending = update_check::check(channel, force).await?;
    emit_update_event(&app, pending.as_ref());
    Ok(pending)
}

#[tauri::command]
fn dismiss_update() -> Result<(), String> {
    update_check::dismiss_current()
}

#[tauri::command]
fn open_release_url(url: String) -> Result<(), String> {
    update_check::open_release_url(&url)
}

fn emit_update_event(app: &tauri::AppHandle, pending: Option<&PendingUpdate>) {
    match pending {
        Some(p) => {
            let _ = app.emit("qmonitor://update-available", p);
        }
        None => {
            let _ = app.emit("qmonitor://update-clear", ());
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    crate::health::init_tracing();

    let app_state = Arc::new(AppState::new());
    let login_listener = LoginListener(Arc::new(Mutex::new(None)));
    let login_attempt = LoginAttemptState(Arc::new(Mutex::new(None)));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .manage(app_state.clone())
        .manage(login_listener)
        .manage(login_attempt)
        .setup(move |app| {
            let state = app_state.clone();
            let handle = app.handle().clone();

            let show_i = MenuItem::with_id(app, "show", "Show qMonitor", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &quit_i])?;
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip("qMonitor")
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            crate::runtime::spawn_workers(handle.clone(), state.clone(), _tray);

            let detectable_state = app_state.clone();
            tauri::async_runtime::spawn(async move {
                detectable_state.refresh_detectable(false).await;
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)).await;
                    detectable_state.refresh_detectable(false).await;
                }
            });

            let update_state = app_state.clone();
            let update_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let channel = update_state.config.read().await.update_channel;
                    match update_check::check(channel, false).await {
                        Ok(pending) => emit_update_event(&update_handle, pending.as_ref()),
                        Err(e) => tracing::warn!(%e, "update check failed"),
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(
                        update_check::CHECK_INTERVAL_SECS,
                    ))
                    .await;
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            let Some(state) = window.try_state::<Arc<AppState>>() else {
                return;
            };
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    let close_to_tray = state
                        .config
                        .try_read()
                        .map(|c| c.close_to_tray)
                        .unwrap_or(false);
                    if close_to_tray {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
                WindowEvent::Resized(_) => {
                    let minimize_to_tray = state
                        .config
                        .try_read()
                        .map(|c| c.minimize_to_tray)
                        .unwrap_or(false);
                    if minimize_to_tray && window.is_minimized().unwrap_or(false) {
                        let _ = window.hide();
                        let _ = window.unminimize();
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            test_base_url,
            get_auth_state,
            start_login,
            cancel_login,
            complete_login,
            set_dev_token,
            sign_out,
            get_home,
            list_games,
            list_pending,
            confirm_game,
            ignore_game,
            unignore_game,
            add_manual_game,
            open_db,
            open_log_dir,
            is_onboarded,
            login_listening,
            get_app_version,
            check_for_updates,
            dismiss_update,
            open_release_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running qMonitor");
}
