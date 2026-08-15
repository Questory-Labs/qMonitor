//! Isolated detect / persist / push / health workers.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tauri::Emitter;
use tokio::sync::{mpsc, watch};

use crate::auth;
use crate::db::SessionRow;
use crate::detect::{foreground_pid, primary_identity, snapshot_processes};
use crate::health::RuntimeHealth;
use crate::identity::ProcessSnapshot;
use crate::live_session::DetectSample;
use crate::persist::{self, PersistCmd};
use crate::push::WebhookClient;
use crate::session::AppState;

const DETECT_BLOCKING_TIMEOUT: Duration = Duration::from_secs(3);
const PUSH_POLL: Duration = Duration::from_secs(2);
const HEALTH_PULSE: Duration = Duration::from_secs(3);

pub fn spawn_workers(app: tauri::AppHandle, state: Arc<AppState>, tray: tauri::tray::TrayIcon) {
    let (sample_tx, sample_rx) = watch::channel(DetectSample::empty());
    let (cmd_tx, cmd_rx) = mpsc::channel::<PersistCmd>(32);
    let (push_tx, push_rx) = mpsc::channel::<SessionRow>(1);
    let (push_result_tx, push_result_rx) = mpsc::channel::<(String, Result<(), String>)>(8);

    {
        let mut slot = state.persist_tx.lock().expect("persist_tx");
        *slot = Some(cmd_tx);
    }

    let detect_state = state.clone();
    tauri::async_runtime::spawn(async move {
        run_detect(detect_state, sample_tx).await;
    });

    let persist_state = state.clone();
    tauri::async_runtime::spawn(async move {
        persist::run_persist(persist_state, sample_rx, cmd_rx, push_tx, push_result_rx).await;
    });

    let push_state = state.clone();
    tauri::async_runtime::spawn(async move {
        run_push(push_state, push_rx, push_result_tx).await;
    });

    let health_state = state.clone();
    tauri::async_runtime::spawn(async move {
        run_health(app, health_state, tray).await;
    });
}

async fn run_detect(state: Arc<AppState>, sample_tx: watch::Sender<DetectSample>) {
    let mut prev_processes: Vec<ProcessSnapshot> = Vec::new();
    let mut prev_fg: Option<u32> = None;
    let mut in_flight: Option<tokio::task::JoinHandle<(Vec<ProcessSnapshot>, Option<u32>)>> = None;
    loop {
        let interval = state.config.read().await.poll_interval_secs.max(1);

        let handle = if let Some(h) = in_flight.take() {
            h
        } else {
            tokio::task::spawn_blocking(|| {
                let processes = snapshot_processes();
                let fg = foreground_pid();
                (processes, fg)
            })
        };

        let snap = tokio::time::timeout(DETECT_BLOCKING_TIMEOUT, handle).await;

        let (processes, fg) = match snap {
            Ok(Ok(pair)) => {
                prev_processes = pair.0.clone();
                prev_fg = pair.1;
                pair
            }
            Ok(Err(e)) => {
                tracing::warn!(%e, "detect join failed");
                (prev_processes.clone(), prev_fg)
            }
            Err(timeout_handle) => {
                state.health.write().await.detect_timeouts += 1;
                tracing::warn!("detect snapshot timed out; keeping previous sample");
                in_flight = Some(timeout_handle.into_inner());
                (prev_processes.clone(), prev_fg)
            }
        };

        let (identities, pending) = {
            let pipe = state.pipeline.read().await;
            pipe.resolve_running(&processes)
        };
        *state.pending_detections.write().await = pending;
        let primary = primary_identity(&identities, fg, &processes).cloned();

        if let Some(p) = &primary {
            let pid = processes
                .iter()
                .find(|proc| {
                    p.exe
                        .as_ref()
                        .map(|e| e.eq_ignore_ascii_case(&proc.name))
                        .unwrap_or(false)
                })
                .map(|proc| proc.pid);
            tracing::debug!(
                id = %p.id,
                title = %p.title,
                ?pid,
                "detect primary"
            );
        }

        let sample = DetectSample {
            observed_at: Utc::now(),
            primary,
        };
        state.live.write().await.apply(&sample);
        state.health.write().await.last_detect_at = Some(sample.observed_at);
        let _ = sample_tx.send(sample);

        tokio::time::sleep(Duration::from_secs(interval)).await;
    }
}

async fn run_push(
    state: Arc<AppState>,
    mut push_rx: mpsc::Receiver<SessionRow>,
    result_tx: mpsc::Sender<(String, Result<(), String>)>,
) {
    let client = WebhookClient::new();
    loop {
        let cfg = state.config.read().await.clone();
        let can_push = cfg.webhook_url().is_some() && auth::get_access_token(&cfg).is_some();

        if !can_push {
            tokio::time::sleep(PUSH_POLL).await;
            continue;
        }

        tokio::select! {
            Some(row) = push_rx.recv() => {
                let cfg = state.config.read().await.clone();
                let result = match (cfg.webhook_url(), auth::get_access_token(&cfg)) {
                    (Some(url), Some(token)) => client.push(&cfg, &url, &token, &row).await,
                    _ => Err("webhook or token missing".to_string()),
                };
                if result.is_ok() {
                    state.health.write().await.last_push_at = Some(Utc::now());
                } else if let Err(e) = &result {
                    *state.last_error.write().await = Some(e.clone());
                }
                let _ = result_tx.send((row.id, result)).await;
            }
            _ = tokio::time::sleep(PUSH_POLL) => {
                continue;
            }
            else => break,
        }
    }
}

async fn run_health(app: tauri::AppHandle, state: Arc<AppState>, tray: tauri::tray::TrayIcon) {
    let mut last_prune = tokio::time::Instant::now();
    loop {
        let (interval, log_level) = {
            let cfg = state.config.read().await;
            (cfg.poll_interval_secs.max(1), cfg.log_level)
        };
        let health: RuntimeHealth = state.health.read().await.clone();
        let tip = health.tray_tooltip(interval);
        let _ = tray.set_tooltip(Some(&tip));
        let _ = app.emit("qmonitor://tick", ());
        if last_prune.elapsed() >= crate::health::LOG_PRUNE_EVERY {
            crate::health::prune_now(log_level);
            last_prune = tokio::time::Instant::now();
        }
        tokio::time::sleep(HEALTH_PULSE).await;
    }
}
