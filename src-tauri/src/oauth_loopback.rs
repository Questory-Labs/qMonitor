//! Local OAuth loopback listener on 127.0.0.1:58473.
//! Port is in the IANA dynamic range (49152–65535) to avoid common service clashes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Deserialize;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::auth::{self, LoginAttempt};
use crate::session::AppState;

pub const LOOPBACK_ADDR: &str = "127.0.0.1:58473";
pub const REDIRECT_URI: &str = "http://127.0.0.1:58473/callback";

static LISTENING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Deserialize)]
struct CodeBody {
    code: String,
    state: String,
}

pub struct LoopbackGuard {
    cancel: Arc<AtomicBool>,
}

impl Drop for LoopbackGuard {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        LISTENING.store(false, Ordering::SeqCst);
    }
}

pub type LoginAttemptSlot = Arc<Mutex<Option<LoginAttempt>>>;
pub type ConfigSlot = Arc<AppState>;

/// Start listening for the OAuth redirect. Returns Err if already listening.
pub async fn start_listener(
    app: AppHandle,
    attempt_slot: LoginAttemptSlot,
    config_slot: ConfigSlot,
) -> Result<LoopbackGuard, String> {
    if LISTENING.swap(true, Ordering::SeqCst) {
        return Err("login listener already running".into());
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_task = cancel.clone();
    let listener = TcpListener::bind(LOOPBACK_ADDR).await.map_err(|e| {
        LISTENING.store(false, Ordering::SeqCst);
        format!("bind {LOOPBACK_ADDR}: {e}")
    })?;

    tauri::async_runtime::spawn(async move {
        while !cancel_task.load(Ordering::SeqCst) {
            let accept = tokio::time::timeout(
                std::time::Duration::from_millis(400),
                listener.accept(),
            )
            .await;
            let Ok(Ok((mut socket, _))) = accept else {
                continue;
            };

            let mut buf = vec![0u8; 8192];
            let n = match socket.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => continue,
            };
            let req = String::from_utf8_lossy(&buf[..n]);
            let (method, path) = parse_request_line(&req);

            if method == "GET"
                && path.starts_with("/callback")
                && !path.starts_with("/callback/complete")
            {
                // Auth-code query on the request line — exchange immediately.
                if path.contains("code=") {
                    if let Ok((code, state)) =
                        auth::parse_callback_code(&format!("http://127.0.0.1{path}"))
                    {
                        match finish_code_exchange(
                            &attempt_slot,
                            &config_slot,
                            &code,
                            &state,
                        )
                        .await
                        {
                            Ok(()) => {
                                let _ = app.emit(
                                    "qmonitor://auth-success",
                                    serde_json::json!({ "ok": true }),
                                );
                                let ok = SUCCESS_HTML;
                                let resp = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    ok.len(),
                                    ok
                                );
                                let _ = socket.write_all(resp.as_bytes()).await;
                                cancel_task.store(true, Ordering::SeqCst);
                                LISTENING.store(false, Ordering::SeqCst);
                                break;
                            }
                            Err(e) => {
                                let msg = format!("Login failed: {e}");
                                let body = format!(
                                    "<!DOCTYPE html><html><body style=\"font-family:system-ui;background:#0f1419;color:#e8eef4;display:grid;place-items:center;min-height:100vh\"><p>{}</p></body></html>",
                                    html_escape(&msg)
                                );
                                let resp = format!(
                                    "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    body.len(),
                                    body
                                );
                                let _ = socket.write_all(resp.as_bytes()).await;
                                continue;
                            }
                        }
                    }
                }

                let body = CALLBACK_HTML;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
                continue;
            }

            if method == "POST" && path.starts_with("/callback/complete") {
                if let Some(json) = req.split("\r\n\r\n").nth(1) {
                    if let Ok(body) = serde_json::from_str::<CodeBody>(json.trim()) {
                        match finish_code_exchange(
                            &attempt_slot,
                            &config_slot,
                            &body.code,
                            &body.state,
                        )
                        .await
                        {
                            Ok(()) => {
                                let _ = app.emit(
                                    "qmonitor://auth-success",
                                    serde_json::json!({ "ok": true }),
                                );
                                let ok = SUCCESS_HTML;
                                let resp = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    ok.len(),
                                    ok
                                );
                                let _ = socket.write_all(resp.as_bytes()).await;
                                cancel_task.store(true, Ordering::SeqCst);
                                LISTENING.store(false, Ordering::SeqCst);
                                break;
                            }
                            Err(_) => {
                                let err = b"HTTP/1.1 400 Bad Request\r\nContent-Length: 11\r\nConnection: close\r\n\r\nbad request";
                                let _ = socket.write_all(err).await;
                                continue;
                            }
                        }
                    }
                }
                let err = b"HTTP/1.1 400 Bad Request\r\nContent-Length: 11\r\nConnection: close\r\n\r\nbad request";
                let _ = socket.write_all(err).await;
                continue;
            }

            let miss = b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found";
            let _ = socket.write_all(miss).await;
        }
        LISTENING.store(false, Ordering::SeqCst);
    });

    Ok(LoopbackGuard { cancel })
}

async fn finish_code_exchange(
    attempt_slot: &LoginAttemptSlot,
    config_slot: &ConfigSlot,
    code: &str,
    state: &str,
) -> Result<(), String> {
    let attempt = {
        let guard = attempt_slot.lock().await;
        guard.clone().ok_or_else(|| "no login attempt".to_string())?
    };
    let cfg = config_slot.config.read().await.clone();
    auth::exchange_authorization_code(&cfg, &attempt, code, state).await?;
    let mut guard = attempt_slot.lock().await;
    *guard = None;
    Ok(())
}

pub fn stop_listener(slot: &Arc<Mutex<Option<LoopbackGuard>>>) {
    if let Ok(mut g) = slot.try_lock() {
        *g = None;
    }
    LISTENING.store(false, Ordering::SeqCst);
}

pub fn is_listening() -> bool {
    LISTENING.load(Ordering::SeqCst)
}

fn parse_request_line(req: &str) -> (String, String) {
    let line = req.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    (method, path)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const CALLBACK_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>qMonitor login</title>
  <style>
    body { font-family: system-ui, sans-serif; background:#0f1419; color:#e8eef4;
           display:grid; place-items:center; min-height:100vh; margin:0; }
    .box { text-align:center; max-width:28rem; padding:2rem; }
    .q { font-size:3rem; font-weight:800; color:#3d9cf0;
         animation: pulse 1.2s ease-in-out infinite; }
    @keyframes pulse { 50% { opacity:0.45; transform:scale(1.06); } }
    p { color:#8b9aab; }
  </style>
</head>
<body>
  <div class="box">
    <div class="q">q</div>
    <p id="msg">Finishing login…</p>
  </div>
  <script>
    (async () => {
      const query = location.search.startsWith('?') ? location.search.slice(1) : '';
      const params = new URLSearchParams(query);
      const code = params.get('code');
      const state = params.get('state');
      const err = params.get('error');
      const el = document.getElementById('msg');
      if (err) {
        el.textContent = 'Authorization declined or failed (' + err + ').';
        return;
      }
      if (!code || !state) {
        el.textContent = 'No authorization code in this URL. Copy the full URL into qMonitor.';
        return;
      }
      try {
        const res = await fetch('/callback/complete', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ code, state })
        });
        if (!res.ok) throw new Error('bad status');
        el.textContent = 'Logged in — you can close this tab and return to qMonitor.';
      } catch (e) {
        el.textContent = 'Could not reach qMonitor. Paste the callback URL into the app.';
      }
    })();
  </script>
</body>
</html>"#;

const SUCCESS_HTML: &str = r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"/><title>qMonitor</title>
<style>body{font-family:system-ui;background:#0f1419;color:#e8eef4;display:grid;place-items:center;min-height:100vh;margin:0}
.q{font-size:3rem;font-weight:800;color:#3ecf8e}</style></head>
<body><div style="text-align:center"><div class="q">q</div>
<p>Login successful. Return to qMonitor.</p></div></body></html>"#;
