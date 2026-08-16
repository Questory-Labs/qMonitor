import { invoke } from "@tauri-apps/api/core";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { QMark } from "./QMark";
import { UpdateSettings, type UpdateChannel } from "./UpdateSettings";

export type LogLevel = "off" | "error" | "warn" | "info" | "debug";

export interface AppConfig {
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
  logLevel?: LogLevel;
  devAccessToken?: string;
}

export interface AuthState {
  baseUrl: string;
  webhookUrl: string;
  hasAccessToken: boolean;
  hasSessionToken: boolean;
}

type LoginPhase = "idle" | "waiting";

function ToggleRow({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="setting-row">
      <span className="setting-row-label">{label}</span>
      <label className="toggle">
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => onChange(e.target.checked)}
          aria-label={label}
        />
        <span className="toggle-ui" />
      </label>
    </div>
  );
}

export function Settings({
  config,
  setConfig,
  auth,
  signedIn,
  loginPhase,
  showManualAuth,
  setShowManualAuth,
  callbackUrl,
  setCallbackUrl,
  saveSettings,
  onStartLogin,
  onCancelLogin,
  onCompleteLogin,
  showToast,
  refresh,
}: {
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
  auth: AuthState | null;
  signedIn: boolean;
  loginPhase: LoginPhase;
  showManualAuth: boolean;
  setShowManualAuth: (v: boolean | ((p: boolean) => boolean)) => void;
  callbackUrl: string;
  setCallbackUrl: (v: string) => void;
  saveSettings: (next: AppConfig) => Promise<void>;
  onStartLogin: () => void;
  onCancelLogin: () => void;
  onCompleteLogin: () => void;
  showToast: (text: string, isError?: boolean) => void;
  refresh: () => Promise<void>;
}) {
  const [autostart, setAutostart] = useState(false);

  useEffect(() => {
    isEnabled()
      .then(setAutostart)
      .catch(() => {
        /* plugin may be unavailable in browser preview */
      });
  }, []);

  async function onAutostart(checked: boolean) {
    try {
      if (checked) await enable();
      else await disable();
      setAutostart(await isEnabled());
      await saveSettings({
        ...config,
        startAtLogin: checked,
      });
    } catch (err) {
      showToast(String(err), true);
    }
  }

  async function openLogs() {
    try {
      const path = await invoke<string>("open_log_dir");
      showToast(`Opened ${path}`);
    } catch (e) {
      showToast(String(e), true);
    }
  }

  async function openDb() {
    try {
      const path = await invoke<string>("open_db");
      showToast(`Opened ${path}`);
    } catch (e) {
      showToast(String(e), true);
    }
  }

  async function browsePath(
    title: string,
    filters: { name: string; extensions: string[] }[],
  ): Promise<string | undefined> {
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        title,
        filters,
      });
      if (typeof selected === "string" && selected) return selected;
    } catch (e) {
      showToast(String(e), true);
    }
  }

  async function browseCatalog() {
    const path = await browsePath("Choose catalog file", [
      { name: "JSON", extensions: ["json"] },
      { name: "All files", extensions: ["*"] },
    ]);
    if (path) setConfig({ ...config, catalogPath: path });
  }

  async function browseDb() {
    const path = await browsePath("Choose database file", [
      { name: "Database", extensions: ["db", "sqlite", "sqlite3"] },
      { name: "All files", extensions: ["*"] },
    ]);
    if (path) setConfig({ ...config, dbPath: path });
  }

  async function signOut() {
    await invoke("sign_out");
    showToast("Signed out");
    await refresh();
  }

  return (
    <div className="settings-stack">
      <section className="settings-card">
        <h2 className="section-label">Account</h2>
        <div className="setting-row">
          <div className="setting-row-text">
            <p className="account-status">
              <span className={`dot ${signedIn ? "on" : ""}`} />
              {signedIn ? "Signed in" : "Not signed in"}
            </p>
            {signedIn ? (
              <span className="setting-row-hint">
                {auth?.webhookUrl ? "Webhook ready" : "Webhook not configured"}
              </span>
            ) : null}
          </div>
          {signedIn ? (
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              onClick={() => void signOut()}
            >
              Sign out
            </button>
          ) : loginPhase !== "waiting" ? (
            <button
              type="button"
              className="btn btn-primary btn-sm"
              onClick={onStartLogin}
              aria-label="Log in with Questory"
            >
              Log in
            </button>
          ) : null}
        </div>
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
            <div className="actions">
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={onCancelLogin}
              >
                Cancel
              </button>
              <button
                type="button"
                className="btn btn-ghost btn-sm"
                onClick={() => setShowManualAuth((v) => !v)}
              >
                Paste callback URL
              </button>
            </div>
          </div>
        ) : null}
        {!signedIn && (showManualAuth || loginPhase === "waiting") ? (
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
                className="btn btn-primary btn-sm"
                onClick={onCompleteLogin}
              >
                Complete login
              </button>
            </div>
          </div>
        ) : null}
      </section>

      <UpdateSettings
        channel={config.updateChannel ?? "stable"}
        onChannelChange={(updateChannel) =>
          saveSettings({ ...config, updateChannel })
        }
        showToast={showToast}
      />

      <section className="settings-card">
        <h2 className="section-label">App</h2>
        <ToggleRow
          label="Start at login"
          checked={autostart}
          onChange={(checked) => void onAutostart(checked)}
        />
        <ToggleRow
          label="Minimize to tray"
          checked={config.minimizeToTray}
          onChange={(checked) =>
            void saveSettings({ ...config, minimizeToTray: checked })
          }
        />
        <ToggleRow
          label="Close to tray"
          checked={config.closeToTray}
          onChange={(checked) =>
            void saveSettings({ ...config, closeToTray: checked })
          }
        />
      </section>

      <section className="settings-card">
        <h2 className="section-label">Monitor</h2>
        <div className="setting-row">
          <div className="setting-row-text">
            <span className="setting-row-label">Poll interval</span>
            <span className="setting-row-hint">Seconds</span>
          </div>
          <div className="setting-row-control">
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
              aria-label="Poll interval in seconds"
            />
          </div>
        </div>
        <div className="setting-row">
          <div className="setting-row-text">
            <span className="setting-row-label">Retention</span>
            <span className="setting-row-hint">Synced sessions</span>
          </div>
          <div className="setting-row-control">
            <select
              value={config.retentionAckedDays}
              onChange={(e) =>
                void saveSettings({
                  ...config,
                  retentionAckedDays: Number(e.target.value),
                })
              }
              aria-label="Retention for synced sessions"
            >
              <option value={7}>7 days</option>
              <option value={30}>30 days</option>
            </select>
          </div>
        </div>
      </section>

      <details className="settings-card settings-advanced">
        <summary>Advanced</summary>

        <div className="advanced-group">
          <h3 className="advanced-group-label">Catalog</h3>
          <div className="field">
            <span>Catalog path</span>
            <div className="path-row">
              <input
                value={config.catalogPath ?? ""}
                onChange={(e) =>
                  setConfig({ ...config, catalogPath: e.target.value })
                }
                placeholder="catalogs/games.example.json"
                aria-label="Catalog path"
              />
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={() => void browseCatalog()}
              >
                Browse
              </button>
            </div>
          </div>
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
        </div>

        <div className="advanced-group">
          <h3 className="advanced-group-label">Logging</h3>
          <div className="field">
            <div className="field-head">
              <span>Log level</span>
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={() => void openLogs()}
              >
                Open folder
              </button>
            </div>
            <select
              value={config.logLevel ?? "off"}
              onChange={(e) =>
                void saveSettings({
                  ...config,
                  logLevel: e.target.value as LogLevel,
                })
              }
              aria-label="Log level"
            >
              <option value="off">Off</option>
              <option value="error">Error</option>
              <option value="warn">Warn</option>
              <option value="info">Info</option>
              <option value="debug">Debug</option>
            </select>
            <p className="setting-row-hint">
              Off by default · 3 days · 5 MB cap
            </p>
          </div>
        </div>

        <div className="advanced-group">
          <h3 className="advanced-group-label">Database</h3>
          <div className="field">
            <div className="field-head">
              <span>Path</span>
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={() => void openDb()}
              >
                Open DB
              </button>
            </div>
            <div className="path-row">
              <input
                value={config.dbPath ?? ""}
                onChange={(e) =>
                  setConfig({ ...config, dbPath: e.target.value })
                }
                placeholder="Default: config dir / qmonitor.db"
                aria-label="Database path"
              />
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={() => void browseDb()}
              >
                Browse
              </button>
            </div>
          </div>
        </div>

        <div className="advanced-group">
          <h3 className="advanced-group-label">Dev</h3>
          <label className="field">
            <span>Access token</span>
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
        </div>
      </details>

      <button
        type="button"
        className="btn btn-primary settings-save"
        onClick={() => void saveSettings(config)}
      >
        Save settings
      </button>
    </div>
  );
}
