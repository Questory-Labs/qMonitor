import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import { useCallback, useEffect, useState } from "react";
import { QMark } from "./components/QMark";
import { UpdateBanner } from "./components/UpdateBanner";
import {
  UpdateSettings,
  type UpdateChannel,
} from "./components/UpdateSettings";
import "./App.css";

type Tab = "home" | "games" | "settings";

type PushStatus = "active" | "pending" | "synced" | "failed";

interface SessionRow {
  id: string;
  identityId: string;
  title: string;
  steamAppId?: number;
  exe?: string;
  source: string;
  startedAt: string;
  endedAt?: string;
  durationSecs?: number;
  pushStatus: PushStatus;
  lastError?: string;
}

interface SyncStatus {
  tursoOk: boolean;
  pendingCount: number;
  lastError?: string;
  activeTitle?: string;
  webhookConfigured: boolean;
}

interface PendingDetection {
  processName: string;
  exePath?: string;
  fingerprint: string;
  suggestedTitle: string;
  identityId?: string;
}

interface HomeState {
  sync: SyncStatus;
  active?: SessionRow;
  history: SessionRow[];
  pendingDetections: PendingDetection[];
}

interface AppConfig {
  baseUrl?: string;
  apiRoot?: string;
  webOrigin?: string;
  service?: "fe" | "be";
  dbPath?: string;
  pollIntervalSecs: number;
  retentionAckedDays: number;
  catalogPath?: string;
  detectableUrl?: string;
  steamPathOverride?: string;
  startAtLogin: boolean;
  minimizeToTray: boolean;
  closeToTray: boolean;
  updateChannel: UpdateChannel;
  devAccessToken?: string;
}

interface TrackableGame {
  id: string;
  title: string;
  steamAppId?: number;
  source: string;
  trackingEnabled: boolean;
}

interface AuthState {
  baseUrl: string;
  webhookUrl: string;
  hasAccessToken: boolean;
  hasSessionToken: boolean;
}

function BrandMark({ size = "sm" }: { size?: "sm" | "md" }) {
  const px = size === "md" ? 36 : 28;
  return (
    <div className="brand-mark">
      <img src="/favicon.svg" alt="" width={px} height={px} />
      <span>qMonitor</span>
    </div>
  );
}

function formatDuration(secs?: number) {
  if (secs == null) return "—";
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

function formatLiveClock(totalSecs: number) {
  const h = Math.floor(totalSecs / 3600);
  const m = Math.floor((totalSecs % 3600) / 60);
  const s = totalSecs % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  if (h > 0) return `${h}:${mm}:${ss}`;
  return `${mm}:${ss}`;
}

function useElapsedSecs(startedAt?: string) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!startedAt) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [startedAt]);
  if (!startedAt) return 0;
  const start = new Date(startedAt).getTime();
  if (Number.isNaN(start)) return 0;
  return Math.max(0, Math.floor((now - start) / 1000));
}

function syncBadge(status: PushStatus) {
  switch (status) {
    case "synced":
      return <span className="badge ok">Synced</span>;
    case "pending":
      return <span className="badge pending">Pending</span>;
    case "failed":
      return <span className="badge fail">Failed</span>;
    case "active":
      return <span className="badge live">Playing</span>;
  }
}

function App() {
  const [tab, setTab] = useState<Tab>("home");
  const [home, setHome] = useState<HomeState | null>(null);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [games, setGames] = useState<TrackableGame[]>([]);
  const [auth, setAuth] = useState<AuthState | null>(null);
  const [onboarded, setOnboarded] = useState(true);
  const [callbackUrl, setCallbackUrl] = useState("");
  const [confirmTitles, setConfirmTitles] = useState<Record<string, string>>(
    {},
  );
  const [message, setMessage] = useState<string | null>(null);
  const [messageIsError, setMessageIsError] = useState(false);
  const [autostart, setAutostart] = useState(false);
  const [loginPhase, setLoginPhase] = useState<"idle" | "waiting">("idle");
  const [showManualAuth, setShowManualAuth] = useState(false);
  const [testingUrl, setTestingUrl] = useState(false);
  const [gamesFilter, setGamesFilter] = useState("");
  const [addOpen, setAddOpen] = useState(false);
  const [addTitle, setAddTitle] = useState("");
  const [addExe, setAddExe] = useState("");
  const [addSteamId, setAddSteamId] = useState("");
  const [adding, setAdding] = useState(false);

  const showToast = useCallback((text: string, isError = false) => {
    setMessageIsError(isError);
    setMessage(text);
  }, []);

  const elapsed = useElapsedSecs(home?.active?.startedAt);

  /** Live status only — never overwrite draft settings while the user is typing. */
  const refresh = useCallback(async () => {
    try {
      const [h, a, o, g] = await Promise.all([
        invoke<HomeState>("get_home"),
        invoke<AuthState | null>("get_auth_state"),
        invoke<boolean>("is_onboarded"),
        invoke<TrackableGame[]>("list_games"),
      ]);
      setHome(h);
      setAuth(a);
      setOnboarded(o);
      setGames(g);
      try {
        setAutostart(await isEnabled());
      } catch {
        /* plugin may be unavailable in browser preview */
      }
    } catch (e) {
      showToast(String(e), true);
    }
  }, [showToast]);

  const loadConfig = useCallback(async () => {
    try {
      setConfig(await invoke<AppConfig>("get_config"));
    } catch (e) {
      showToast(String(e), true);
    }
  }, [showToast]);

  useEffect(() => {
    void loadConfig();
    void refresh();
    const unsubs: Array<() => void> = [];
    listen("qmonitor://tick", () => {
      refresh();
    }).then((fn) => unsubs.push(fn));
    listen("qmonitor://auth-success", async () => {
      setLoginPhase("idle");
      setShowManualAuth(false);
      showToast("Logged in");
      await loadConfig();
      await refresh();
    }).then((fn) => unsubs.push(fn));
    listen("qmonitor://auth-waiting", () => {
      setLoginPhase("waiting");
    }).then((fn) => unsubs.push(fn));
    const id = setInterval(refresh, 5000);
    return () => {
      clearInterval(id);
      unsubs.forEach((u) => u());
    };
  }, [refresh, loadConfig, showToast]);

  useEffect(() => {
    if (!message) return;
    const ms = messageIsError ? 5000 : 3500;
    const id = setTimeout(() => setMessage(null), ms);
    return () => clearTimeout(id);
  }, [message, messageIsError]);

  async function saveAndTest() {
    if (!config) return;
    setTestingUrl(true);
    try {
      const saved = await invoke<AppConfig>("save_config", { config });
      setConfig(saved);
      const status = await invoke<string>("test_base_url", {
        baseUrl: saved.baseUrl ?? "",
      });
      showToast(`Reachable — ${status}`);
      await refresh();
    } catch (e) {
      showToast(String(e), true);
    } finally {
      setTestingUrl(false);
    }
  }

  async function saveSettings(next: AppConfig) {
    try {
      const saved = await invoke<AppConfig>("save_config", { config: next });
      setConfig(saved);
      showToast("Saved");
      await refresh();
    } catch (e) {
      showToast(String(e), true);
    }
  }

  async function onStartLogin() {
    try {
      if (config) {
        await invoke("save_config", { config });
      }
      setLoginPhase("waiting");
      setShowManualAuth(false);
      await invoke<string>("start_login");
    } catch (e) {
      setLoginPhase("idle");
      showToast(String(e), true);
    }
  }

  async function onCancelLogin() {
    try {
      await invoke("cancel_login");
    } catch {
      /* ignore */
    }
    setLoginPhase("idle");
  }

  async function onCompleteLogin() {
    try {
      const a = await invoke<AuthState>("complete_login", {
        callbackUrl,
      });
      setAuth(a);
      setLoginPhase("idle");
      setShowManualAuth(false);
      showToast("Logged in");
      await refresh();
    } catch (e) {
      showToast(String(e), true);
    }
  }

  async function setTracking(game: TrackableGame, enabled: boolean) {
    try {
      if (enabled) {
        await invoke("unignore_game", { identityId: game.id });
      } else {
        await invoke("ignore_game", {
          identityId: game.id,
          title: game.title,
        });
      }
      await refresh();
    } catch (e) {
      showToast(String(e), true);
    }
  }

  function openAddDialog() {
    setAddTitle("");
    setAddExe("");
    setAddSteamId("");
    setAddOpen(true);
  }

  async function submitAddGame() {
    const title = addTitle.trim();
    const exePath = addExe.trim();
    if (!title || !exePath) {
      showToast("Title and exe / path are required", true);
      return;
    }
    let steamAppId: number | undefined;
    const rawId = addSteamId.trim();
    if (rawId) {
      const n = Number(rawId);
      if (!Number.isInteger(n) || n <= 0) {
        showToast("Steam App ID must be a positive number", true);
        return;
      }
      steamAppId = n;
    }
    setAdding(true);
    try {
      await invoke("add_manual_game", {
        title,
        exePath,
        steamAppId: steamAppId ?? null,
      });
      setAddOpen(false);
      showToast("Game added");
      await refresh();
    } catch (e) {
      showToast(String(e), true);
    } finally {
      setAdding(false);
    }
  }

  if (!config) {
    return (
      <div className="shell loading" role="status" aria-label="Loading qMonitor">
        <QMark variant="loading" />
      </div>
    );
  }

  const needsOnboarding = !onboarded;
  const sync = home?.sync;
  const signedIn = Boolean(auth?.hasAccessToken);
  const filterQ = gamesFilter.trim().toLowerCase();
  const filteredGames = filterQ
    ? games.filter(
        (g) =>
          g.title.toLowerCase().includes(filterQ) ||
          g.id.toLowerCase().includes(filterQ),
      )
    : games;
  const history = (home?.history ?? []).filter((s) => s.pushStatus !== "active");

  return (
    <div className="shell">
      <UpdateBanner />
      <div className="scroll-body">
        {needsOnboarding ? (
          <div className="onboard-screen">
            <div className="onboard-col">
              <BrandMark size="md" />
              <p className="eyebrow">Desktop monitor</p>
              <h1>Connect to Questory</h1>
              <p className="lede">
                Point qMonitor at your Questory instance, then sign in to start
                tracking game sessions.
              </p>

              {loginPhase === "waiting" ? (
                <div className="login-wait">
                  <QMark variant="loading" />
                  <h2 className="wait-title">Waiting for Questory…</h2>
                  <p className="lede">
                    Finish login in your browser. Listening on{" "}
                    <code>127.0.0.1:58473</code> — you&apos;ll be signed in
                    automatically.
                  </p>
                  <div className="actions" style={{ justifyContent: "center" }}>
                    <button
                      type="button"
                      className="btn btn-secondary"
                      onClick={onCancelLogin}
                    >
                      Cancel
                    </button>
                    <button
                      type="button"
                      className="btn btn-ghost"
                      onClick={() => setShowManualAuth((v) => !v)}
                    >
                      {showManualAuth ? "Hide paste" : "Paste callback URL"}
                    </button>
                  </div>
                  {showManualAuth ? (
                    <div className="manual-auth">
                      <label className="field">
                        <span>Callback URL (with code=)</span>
                        <input
                          value={callbackUrl}
                          onChange={(e) => setCallbackUrl(e.target.value)}
                          placeholder="Full callback URL containing code="
                          autoFocus
                        />
                      </label>
                      <div className="actions">
                        <button
                          type="button"
                          className="btn btn-primary"
                          onClick={onCompleteLogin}
                        >
                          Complete login
                        </button>
                      </div>
                    </div>
                  ) : null}
                </div>
              ) : (
                <>
                  <label className="field">
                    <span>Questory URL</span>
                    <input
                      value={config.baseUrl ?? ""}
                      onChange={(e) =>
                        setConfig({ ...config, baseUrl: e.target.value })
                      }
                      placeholder="https://app.questorylabs.com"
                    />
                  </label>
                  <div className="actions">
                    <button
                      type="button"
                      className="btn btn-primary"
                      onClick={saveAndTest}
                      disabled={testingUrl}
                    >
                      {testingUrl ? "Testing…" : "Save & test"}
                    </button>
                    <button
                      type="button"
                      className="btn btn-secondary"
                      onClick={onStartLogin}
                    >
                      Log in with Questory
                    </button>
                  </div>
                </>
              )}
            </div>
          </div>
        ) : (
          <div className="app-frame">
            <nav className="tabs">
              {(["home", "games", "settings"] as Tab[]).map((t) => (
                <button
                  key={t}
                  type="button"
                  className={tab === t ? "active" : ""}
                  onClick={() => setTab(t)}
                >
                  {t[0].toUpperCase() + t.slice(1)}
                </button>
              ))}
            </nav>

            <main className="content">
              {tab === "home" && (
                <section className="panel">
                  <h2 className="section-label">Now playing</h2>
                  {home?.active ? (
                    <div className="active-session">
                      <div className="active-session-top">
                        <strong className="active-title">
                          {home.active.title}
                        </strong>
                        {syncBadge("active")}
                      </div>
                      <div className="live-timer" aria-live="polite">
                        {formatLiveClock(elapsed)}
                      </div>
                      <div className="meta">
                        Started{" "}
                        {new Date(home.active.startedAt).toLocaleString()}
                      </div>
                      <div className="active-session-actions">
                        <button
                          type="button"
                          className="btn btn-ghost btn-sm"
                          onClick={async () => {
                            const session = home.active;
                            if (!session) return;
                            try {
                              await invoke("ignore_game", {
                                identityId: session.identityId,
                                title: session.title,
                              });
                              showToast(`Not tracking ${session.title}`);
                              await refresh();
                            } catch (e) {
                              showToast(String(e), true);
                            }
                          }}
                        >
                          Don&apos;t track
                        </button>
                      </div>
                    </div>
                  ) : (
                    <p className="empty-state">No game playing</p>
                  )}

                  <h2 className="section-label">Recent</h2>
                  {history.length === 0 ? (
                    <p className="empty-state">No sessions yet</p>
                  ) : (
                    <ul className="session-list">
                      {history.map((s) => (
                        <li key={s.id} className="row-item">
                          <div>
                            <strong>{s.title}</strong>
                            <div className="meta">
                              {formatDuration(s.durationSecs)} ·{" "}
                              {s.endedAt
                                ? new Date(s.endedAt).toLocaleString()
                                : "—"}
                            </div>
                          </div>
                          {syncBadge(s.pushStatus)}
                        </li>
                      ))}
                    </ul>
                  )}
                </section>
              )}

              {tab === "games" && (
                <section className="panel">
                  <div className="section-row">
                    <h2 className="section-label">Needs confirmation</h2>
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      onClick={openAddDialog}
                    >
                      Add game
                    </button>
                  </div>
                  {(home?.pendingDetections ?? []).length === 0 ? (
                    <p className="empty-state">No games waiting</p>
                  ) : (
                    <ul className="session-list">
                      {home!.pendingDetections.map((p) => (
                        <li key={p.fingerprint} className="row-item pending-row">
                          <div className="pending-body">
                            <strong>{p.suggestedTitle}</strong>
                            <div className="meta">
                              {p.processName}
                              {p.exePath ? ` · ${p.exePath}` : ""}
                            </div>
                            <input
                              value={
                                confirmTitles[p.fingerprint] ??
                                p.suggestedTitle
                              }
                              onChange={(e) =>
                                setConfirmTitles({
                                  ...confirmTitles,
                                  [p.fingerprint]: e.target.value,
                                })
                              }
                              aria-label="Game title"
                            />
                          </div>
                          <div className="row-actions">
                            <button
                              type="button"
                              className="btn btn-primary"
                              onClick={async () => {
                                try {
                                  await invoke("confirm_game", {
                                    fingerprint: p.fingerprint,
                                    title:
                                      confirmTitles[p.fingerprint] ??
                                      p.suggestedTitle,
                                  });
                                  await refresh();
                                } catch (e) {
                                  showToast(String(e), true);
                                }
                              }}
                            >
                              Confirm
                            </button>
                            <button
                              type="button"
                              className="btn btn-ghost"
                              onClick={async () => {
                                const identityId =
                                  p.identityId ?? `user:${p.fingerprint}`;
                                try {
                                  await invoke("ignore_game", {
                                    identityId,
                                    title:
                                      confirmTitles[p.fingerprint] ??
                                      p.suggestedTitle,
                                  });
                                  await refresh();
                                } catch (e) {
                                  showToast(String(e), true);
                                }
                              }}
                            >
                              Don&apos;t track
                            </button>
                          </div>
                        </li>
                      ))}
                    </ul>
                  )}

                  <h2 className="section-label">Tracking</h2>
                  <label className="field games-search">
                    <span className="sr-only">Search games</span>
                    <input
                      value={gamesFilter}
                      onChange={(e) => setGamesFilter(e.target.value)}
                      placeholder="Search library…"
                    />
                  </label>
                  {filteredGames.length === 0 ? (
                    <p className="empty-state">No games found</p>
                  ) : (
                    <ul className="session-list compact tracking-list">
                      {filteredGames.slice(0, 300).map((g) => (
                        <li key={g.id} className="row-item track-row">
                          <div>
                            <span className="track-title">{g.title}</span>
                            <div className="meta">
                              {g.source}
                              {!g.trackingEnabled ? " · off" : ""}
                            </div>
                          </div>
                          <label className="toggle">
                            <input
                              type="checkbox"
                              checked={g.trackingEnabled}
                              onChange={(e) =>
                                void setTracking(g, e.target.checked)
                              }
                              aria-label={`Track ${g.title}`}
                            />
                            <span className="toggle-ui" />
                          </label>
                        </li>
                      ))}
                    </ul>
                  )}
                </section>
              )}

              {tab === "settings" && (
                <section className="panel">
                  <UpdateSettings
                    channel={config.updateChannel ?? "stable"}
                    onChannelChange={(updateChannel) =>
                      saveSettings({ ...config, updateChannel })
                    }
                    showToast={showToast}
                  />
                  <h2 className="section-label">Account</h2>
                  {signedIn ? (
                    <div className="account-card">
                      <p className="account-status">
                        <span className="dot on" />
                        Signed in
                      </p>
                      <p className="meta">
                        {auth?.webhookUrl
                          ? `Webhook ready`
                          : "Webhook not configured"}
                      </p>
                      <div className="actions">
                        <button
                          type="button"
                          className="btn btn-secondary"
                          onClick={async () => {
                            await invoke("sign_out");
                            showToast("Signed out");
                            await refresh();
                          }}
                        >
                          Sign out
                        </button>
                      </div>
                    </div>
                  ) : (
                    <>
                      <p className="meta">Not signed in</p>
                      <div className="actions">
                        <button
                          type="button"
                          className="btn btn-primary"
                          onClick={onStartLogin}
                        >
                          Log in with Questory
                        </button>
                      </div>
                    </>
                  )}

                  <label className="field">
                    <span>Base URL</span>
                    <input
                      value={config.baseUrl ?? ""}
                      onChange={(e) =>
                        setConfig({ ...config, baseUrl: e.target.value })
                      }
                    />
                  </label>
                  {!signedIn && loginPhase === "waiting" ? (
                    <div className="login-wait">
                      <QMark variant="loading" />
                      <p className="lede">
                        Waiting on <code>127.0.0.1:58473</code>
                      </p>
                      <div
                        className="actions"
                        style={{ justifyContent: "center" }}
                      >
                        <button
                          type="button"
                          className="btn btn-secondary"
                          onClick={onCancelLogin}
                        >
                          Cancel
                        </button>
                        <button
                          type="button"
                          className="btn btn-ghost"
                          onClick={() => setShowManualAuth((v) => !v)}
                        >
                          Paste callback URL
                        </button>
                      </div>
                    </div>
                  ) : null}
                  {!signedIn &&
                    (showManualAuth || loginPhase === "waiting") && (
                      <div className="manual-auth">
                        <label className="field">
                          <span>Callback URL (with code=)</span>
                          <input
                            value={callbackUrl}
                            onChange={(e) => setCallbackUrl(e.target.value)}
                            placeholder="Full callback URL containing code="
                          />
                        </label>
                        <div className="actions">
                          <button
                            type="button"
                            className="btn btn-primary"
                            onClick={onCompleteLogin}
                          >
                            Complete login
                          </button>
                        </div>
                      </div>
                    )}

                  <h2 className="section-label">Local database</h2>
                  <label className="field">
                    <span>Database path (optional)</span>
                    <input
                      value={config.dbPath ?? ""}
                      onChange={(e) =>
                        setConfig({ ...config, dbPath: e.target.value })
                      }
                      placeholder="Default: config dir / qmonitor.db"
                    />
                  </label>
                  <div className="actions">
                    <button
                      type="button"
                      className="btn btn-secondary"
                      onClick={async () => {
                        try {
                          const path = await invoke<string>("open_db");
                          showToast(`Opened ${path}`);
                          await refresh();
                        } catch (e) {
                          showToast(String(e), true);
                        }
                      }}
                    >
                      Open / reconnect DB
                    </button>
                  </div>

                  <h2 className="section-label">Monitor</h2>
                  <label className="field">
                    <span>Poll interval (seconds)</span>
                    <input
                      type="number"
                      min={1}
                      value={config.pollIntervalSecs}
                      onChange={(e) =>
                        setConfig({
                          ...config,
                          pollIntervalSecs: Number(e.target.value) || 3,
                        })
                      }
                    />
                  </label>
                  <label className="field">
                    <span>Retention for synced sessions</span>
                    <select
                      value={config.retentionAckedDays}
                      onChange={(e) =>
                        setConfig({
                          ...config,
                          retentionAckedDays: Number(e.target.value),
                        })
                      }
                    >
                      <option value={7}>7 days</option>
                      <option value={30}>30 days</option>
                    </select>
                  </label>
                  <label className="field">
                    <span>Catalog path</span>
                    <input
                      value={config.catalogPath ?? ""}
                      onChange={(e) =>
                        setConfig({ ...config, catalogPath: e.target.value })
                      }
                      placeholder="catalogs/games.example.json"
                    />
                  </label>
                  <label className="field">
                    <span>Detectable catalog URL</span>
                    <input
                      value={config.detectableUrl ?? ""}
                      onChange={(e) =>
                        setConfig({ ...config, detectableUrl: e.target.value })
                      }
                      placeholder="https://discord.com/api/v10/applications/detectable"
                    />
                  </label>
                  <label className="field check">
                    <input
                      type="checkbox"
                      checked={autostart}
                      onChange={async (e) => {
                        try {
                          if (e.target.checked) await enable();
                          else await disable();
                          setAutostart(await isEnabled());
                          await saveSettings({
                            ...config,
                            startAtLogin: e.target.checked,
                          });
                        } catch (err) {
                          showToast(String(err), true);
                        }
                      }}
                    />
                    <span>Start with system login</span>
                  </label>
                  <label className="field check">
                    <input
                      type="checkbox"
                      checked={config.minimizeToTray}
                      onChange={async (e) => {
                        await saveSettings({
                          ...config,
                          minimizeToTray: e.target.checked,
                        });
                      }}
                    />
                    <span>Minimize goes to tray</span>
                  </label>
                  <label className="field check">
                    <input
                      type="checkbox"
                      checked={config.closeToTray}
                      onChange={async (e) => {
                        await saveSettings({
                          ...config,
                          closeToTray: e.target.checked,
                        });
                      }}
                    />
                    <span>Close goes to tray</span>
                  </label>
                  <label className="field">
                    <span>Dev access token</span>
                    <input
                      type="password"
                      value={config.devAccessToken ?? ""}
                      onChange={(e) =>
                        setConfig({
                          ...config,
                          devAccessToken: e.target.value,
                        })
                      }
                    />
                  </label>
                  <div className="actions">
                    <button
                      type="button"
                      className="btn btn-primary"
                      onClick={() => saveSettings(config)}
                    >
                      Save settings
                    </button>
                  </div>
                </section>
              )}
            </main>
          </div>
        )}
      </div>

      <footer className="bottombar">
        <div className="sync-chips">
          <span className="chip" title="Local session database">
            <span className={`dot ${sync?.tursoOk ? "on" : ""}`} />
            {sync?.tursoOk ? "DB" : "DB off"}
          </span>
          <span className="chip">{sync?.pendingCount ?? 0} pending</span>
          {!sync?.webhookConfigured ? (
            <span className="chip warn">no webhook</span>
          ) : null}
          {sync?.lastError ? (
            <span className="chip err" title={sync.lastError}>
              error
            </span>
          ) : null}
        </div>
      </footer>

      {message ? (
        <div
          className={`toast ${messageIsError ? "err" : ""}`}
          onClick={() => setMessage(null)}
          role="status"
        >
          {message}
        </div>
      ) : null}

      {addOpen ? (
        <div
          className="dialog-backdrop"
          role="presentation"
          onClick={() => !adding && setAddOpen(false)}
        >
          <div
            className="dialog"
            role="dialog"
            aria-labelledby="add-game-title"
            onClick={(e) => e.stopPropagation()}
          >
            <h2 id="add-game-title">Add game</h2>
            <p className="lede">
              Track a title that Steam / Discord didn&apos;t pick up. When this
              exe is running, qMonitor will start a session.
            </p>
            <label className="field">
              <span>Name</span>
              <input
                value={addTitle}
                onChange={(e) => setAddTitle(e.target.value)}
                placeholder="Hades"
                autoFocus
              />
            </label>
            <label className="field">
              <span>Exe or full path</span>
              <input
                value={addExe}
                onChange={(e) => setAddExe(e.target.value)}
                placeholder="D:\Games\Hades\Hades.exe"
              />
            </label>
            <label className="field">
              <span>Steam App ID (optional)</span>
              <input
                value={addSteamId}
                onChange={(e) => setAddSteamId(e.target.value)}
                placeholder="1145360"
                inputMode="numeric"
              />
            </label>
            <div className="actions dialog-actions">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => setAddOpen(false)}
                disabled={adding}
              >
                Cancel
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => void submitAddGame()}
                disabled={adding}
              >
                {adding ? "Adding…" : "Add"}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

export default App;
