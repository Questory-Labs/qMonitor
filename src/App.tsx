import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
import { Dialog } from "./components/Dialog";
import { EmptyFace } from "./components/EmptyFace";
import { HatchShadow } from "./components/HatchShadow";
import { OnboardScreen } from "./components/OnboardScreen";
import { RecentSessions } from "./components/RecentSessions";
import { QMark } from "./components/QMark";
import {
  Settings,
  type AppConfig,
  type AuthState,
} from "./components/Settings";
import { UpdateBanner } from "./components/UpdateBanner";
import { invokeTimeout } from "./invoke-timeout";
import {
  isBrowserPreview,
  PREVIEW_AUTH,
  PREVIEW_CONFIG,
  PREVIEW_GAMES,
  PREVIEW_HOME,
  previewOnboarded,
} from "./preview-state";
import "./App.css";

type Tab = "home" | "games" | "settings";

type PushStatus = "active" | "pending" | "synced" | "failed" | "skipped";

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
  lastTickAt?: string;
  loopAlive?: boolean;
  dbReconnects?: number;
  detectTimeouts?: number;
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

interface TrackableGame {
  id: string;
  title: string;
  steamAppId?: number;
  source: string;
  trackingEnabled: boolean;
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

function skipLabel(lastError?: string) {
  if (lastError === "too_short") return "Too short";
  if (lastError === "not_on_list") return "Not on list";
  return "Skipped";
}

function syncBadge(status: PushStatus, lastError?: string) {
  switch (status) {
    case "synced":
      return <span className="badge ok">Synced</span>;
    case "pending":
      return <span className="badge pending">Pending</span>;
    case "failed":
      return <span className="badge fail">Failed</span>;
    case "skipped":
      return <span className="badge skip">{skipLabel(lastError)}</span>;
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

  const lastTimeoutToast = useRef<string | null>(null);

  const elapsed = useElapsedSecs(home?.active?.startedAt);

  const applyPreview = useCallback(() => {
    setHome(PREVIEW_HOME);
    setAuth(PREVIEW_AUTH);
    setOnboarded(previewOnboarded());
    setGames(PREVIEW_GAMES);
    setConfig(PREVIEW_CONFIG);
  }, []);

  /** Live status only — never overwrite draft settings while the user is typing. */
  const refresh = useCallback(async () => {
    if (isBrowserPreview()) {
      applyPreview();
      return;
    }
    try {
      const [h, a, o, g] = await Promise.all([
        invokeTimeout<HomeState>("get_home"),
        invokeTimeout<AuthState | null>("get_auth_state"),
        invokeTimeout<boolean>("is_onboarded"),
        invokeTimeout<TrackableGame[]>("list_games"),
      ]);
      setHome(h);
      setAuth(a);
      setOnboarded(o);
      setGames(g);
      lastTimeoutToast.current = null;
    } catch (e) {
      const text = String(e);
      const isTimeout = text.includes("timed out");
      if (isTimeout && lastTimeoutToast.current === text) {
        return;
      }
      lastTimeoutToast.current = isTimeout ? text : null;
      showToast(text, true);
    }
  }, [applyPreview, showToast]);

  const loadConfig = useCallback(async () => {
    if (isBrowserPreview()) {
      applyPreview();
      return;
    }
    try {
      setConfig(await invoke<AppConfig>("get_config"));
    } catch (e) {
      showToast(String(e), true);
    }
  }, [applyPreview, showToast]);

  useEffect(() => {
    void loadConfig();
    void refresh();
    if (isBrowserPreview()) {
      const params = new URLSearchParams(window.location.search);
      const tabParam = params.get("tab");
      if (tabParam === "home" || tabParam === "games" || tabParam === "settings") {
        setTab(tabParam);
      }
      if (params.has("add")) setAddOpen(true);
      return;
    }
    const unsubs: Array<() => void> = [];
    const onTick = () => {
      if (typeof document !== "undefined" && document.hidden) return;
      void refresh();
    };
    listen("qmonitor://tick", onTick).then((fn) => unsubs.push(fn));
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
    const onVis = () => {
      if (!document.hidden) void refresh();
    };
    document.addEventListener("visibilitychange", onVis);
    const id = setInterval(() => {
      if (!document.hidden) void refresh();
    }, 5000);
    return () => {
      clearInterval(id);
      document.removeEventListener("visibilitychange", onVis);
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

  function titleFromExePath(path: string): string {
    const base = path.replace(/[/\\]+$/, "").split(/[/\\]/).pop() ?? "";
    return base.replace(/\.exe$/i, "").trim();
  }

  async function browseExe() {
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        title: "Choose game executable",
        filters: [
          { name: "Executables", extensions: ["exe"] },
          { name: "All files", extensions: ["*"] },
        ],
      });
      if (typeof selected !== "string" || !selected) return;
      setAddExe(selected);
      setAddTitle((current) => current.trim() || titleFromExePath(selected));
    } catch (e) {
      showToast(String(e), true);
    }
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
          <OnboardScreen
            config={config}
            setConfig={setConfig}
            loginPhase={loginPhase}
            showManualAuth={showManualAuth}
            setShowManualAuth={setShowManualAuth}
            callbackUrl={callbackUrl}
            setCallbackUrl={setCallbackUrl}
            testingUrl={testingUrl}
            saveAndTest={() => void saveAndTest()}
            onStartLogin={() => void onStartLogin()}
            onCancelLogin={() => void onCancelLogin()}
            onCompleteLogin={() => void onCompleteLogin()}
          />
        ) : (
          <div className="app-frame">
            <div className="chrome">
              <nav className="tabs" aria-label="Primary">
                {(["home", "games", "settings"] as Tab[]).map((t) => (
                  <button
                    key={t}
                    type="button"
                    className="nav-hatch"
                    data-active={tab === t}
                    aria-current={tab === t ? "page" : undefined}
                    onClick={() => setTab(t)}
                  >
                    {t[0].toUpperCase() + t.slice(1)}
                  </button>
                ))}
              </nav>
            </div>

            <main className="content">
              {tab === "home" && (
                <>
                  <h2 className="section-label">Now playing</h2>
                  {home?.active ? (
                    <HatchShadow
                      size="sm"
                      faceClassName="panel-accent active-session"
                    >
                      <div className="active-session-top">
                        <strong className="active-title">
                          {home.active.title}
                        </strong>
                        {syncBadge("active")}
                      </div>
                      <div className="live-timer" aria-live="polite">
                        {formatLiveClock(elapsed)}
                      </div>
                      <div className="active-session-meta">
                        <div className="meta">
                          Started{" "}
                          {new Date(home.active.startedAt).toLocaleString()}
                        </div>
                        <button
                          type="button"
                          className="link-quiet"
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
                    </HatchShadow>
                  ) : (
                    <EmptyFace title="No game playing" />
                  )}

                  <h2 className="section-label">Recent</h2>
                  <RecentSessions
                    history={history}
                    onSync={async (s) => {
                      try {
                        await invokeTimeout("push_session", 12000, {
                          sessionId: s.id,
                        });
                        showToast(`Queued ${s.title}`);
                        await refresh();
                      } catch (e) {
                        showToast(String(e), true);
                        throw e;
                      }
                    }}
                    onIgnore={async (s) => {
                      try {
                        await invokeTimeout("ignore_game", 4000, {
                          identityId: s.identityId,
                          title: s.title,
                        });
                        showToast(`Not tracking ${s.title}`);
                        await refresh();
                      } catch (e) {
                        showToast(String(e), true);
                        throw e;
                      }
                    }}
                  />
                </>
              )}

              {tab === "games" && (
                <>
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
                    <EmptyFace title="No games waiting" />
                  ) : (
                    <div className="panel-outline">
                      <ul className="session-list">
                        {home!.pendingDetections.map((p) => (
                          <li
                            key={p.fingerprint}
                            className="row-item pending-row"
                          >
                            <div className="pending-body">
                              <strong>{p.suggestedTitle}</strong>
                              <div className="meta">
                                {p.processName}
                                {p.exePath ? ` · ${p.exePath}` : ""}
                              </div>
                              <input
                                className="field"
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
                    </div>
                  )}

                  <h2 className="section-label">Tracking</h2>
                  {config?.pushFromListOnly ? (
                    <p className="setting-row-hint tracking-hint">
                      Only Tracking-on games auto-sync to Questory. The switch
                      still means Don&apos;t track (stop recording). Sync held
                      sessions from Home → Recent.
                    </p>
                  ) : null}
                  <label className="label games-search">
                    <span className="sr-only">Search games</span>
                    <input
                      className="field"
                      value={gamesFilter}
                      onChange={(e) => setGamesFilter(e.target.value)}
                      placeholder="Search library…"
                    />
                  </label>
                  {filteredGames.length === 0 ? (
                    <EmptyFace title="No games found" />
                  ) : (
                    <div className="panel-outline">
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
                    </div>
                  )}
                </>
              )}

              {tab === "settings" && (
                <Settings
                  config={config}
                  setConfig={setConfig}
                  auth={auth}
                  signedIn={signedIn}
                  loginPhase={loginPhase}
                  showManualAuth={showManualAuth}
                  setShowManualAuth={setShowManualAuth}
                  callbackUrl={callbackUrl}
                  setCallbackUrl={setCallbackUrl}
                  saveSettings={saveSettings}
                  onStartLogin={onStartLogin}
                  onCancelLogin={onCancelLogin}
                  onCompleteLogin={onCompleteLogin}
                  showToast={showToast}
                  refresh={refresh}
                />
              )}
            </main>
          </div>
        )}
      </div>

      <footer className="status-bar">
        <div className="sync-chips">
          <span
            className="chip"
            title={
              sync?.lastTickAt
                ? `Local session database · last poll ${new Date(sync.lastTickAt).toLocaleString()}${sync.loopAlive === false ? " · loop stuck" : ""}`
                : "Local session database"
            }
          >
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
        <button
          type="button"
          className="toast-wrap"
          onClick={() => setMessage(null)}
        >
          <HatchShadow
            size="sm"
            faceClassName={`dialog-face toast-face${messageIsError ? " toast-err" : ""}`}
          >
            {message}
          </HatchShadow>
        </button>
      ) : null}

      <Dialog
        open={addOpen}
        onClose={() => {
          if (!adding) setAddOpen(false);
        }}
        title="Add game"
      >
        <p className="lede">
          Track a title that Steam / Discord didn&apos;t pick up. When this exe
          is running, qMonitor will start a session.
        </p>
        <label className="label">
          <span>Name</span>
          <input
            className="field"
            value={addTitle}
            onChange={(e) => setAddTitle(e.target.value)}
            placeholder="Hades"
            autoFocus
          />
        </label>
        <div className="label">
          <span>Exe or full path</span>
          <div className="path-row">
            <input
              id="add-exe"
              className="field"
              value={addExe}
              onChange={(e) => setAddExe(e.target.value)}
              placeholder="D:\Games\Hades\Hades.exe"
              aria-label="Exe or full path"
            />
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => void browseExe()}
              disabled={adding}
            >
              Browse
            </button>
          </div>
        </div>
        <label className="label">
          <span>Steam App ID (optional)</span>
          <input
            className="field"
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
      </Dialog>
    </div>
  );
}

export default App;
