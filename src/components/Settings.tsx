import { invoke } from "@tauri-apps/api/core";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { useEffect, useState, type ReactNode } from "react";
import { AdvancedSettings } from "./AdvancedSettings";
import { HatchShadow } from "./HatchShadow";
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
  minPushDurationMins?: number;
  pushFromListOnly?: boolean;
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

function SettingsCard({ children }: { children: ReactNode }) {
  return (
    <HatchShadow size="sm" faceClassName="panel settings-face">
      {children}
    </HatchShadow>
  );
}

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

  async function signOut() {
    await invoke("sign_out");
    showToast("Signed out");
    await refresh();
  }

  return (
    <div className="settings-stack">
      <SettingsCard>
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
        <label className="label">
          <span>Base URL</span>
          <input
            className="field"
            value={config.baseUrl ?? ""}
            onChange={(e) => setConfig({ ...config, baseUrl: e.target.value })}
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
            <label className="label">
              <span>Callback URL (with code=)</span>
              <input
                className="field"
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
      </SettingsCard>

      <UpdateSettings
        channel={config.updateChannel ?? "stable"}
        onChannelChange={(updateChannel) =>
          saveSettings({ ...config, updateChannel })
        }
        showToast={showToast}
      />

      <SettingsCard>
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
      </SettingsCard>

      <SettingsCard>
        <h2 className="section-label">Monitor</h2>
        <div className="setting-row">
          <div className="setting-row-text">
            <span className="setting-row-label">Poll interval</span>
            <span className="setting-row-hint">Seconds</span>
          </div>
          <div className="setting-row-control">
            <input
              className="field field-sm"
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
              className="field field-sm"
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
        <div className="setting-row">
          <div className="setting-row-text">
            <span className="setting-row-label">Minimum session</span>
            <span className="setting-row-hint">Minutes · 0 = report all</span>
          </div>
          <div className="setting-row-control">
            <input
              className="field field-sm"
              type="number"
              min={0}
              value={config.minPushDurationMins ?? 0}
              onChange={(e) => {
                const n = Number(e.target.value);
                setConfig({
                  ...config,
                  minPushDurationMins:
                    Number.isFinite(n) && n >= 0 ? Math.floor(n) : 0,
                });
              }}
              aria-label="Minimum session duration in minutes"
            />
          </div>
        </div>
        <div className="setting-row">
          <div className="setting-row-text">
            <span className="setting-row-label">
              Only auto-sync games that are Tracking on
            </span>
            <span className="setting-row-hint setting-row-hint--wrap">
              Other titles stay local. Sync them from Home → Recent. The Games
              switch still means Don&apos;t track.
            </span>
          </div>
          <label className="toggle">
            <input
              type="checkbox"
              checked={config.pushFromListOnly ?? false}
              onChange={(e) =>
                void saveSettings({
                  ...config,
                  pushFromListOnly: e.target.checked,
                })
              }
              aria-label="Only auto-sync games that are Tracking on"
            />
            <span className="toggle-ui" />
          </label>
        </div>
      </SettingsCard>

      <AdvancedSettings
        config={config}
        setConfig={setConfig}
        saveSettings={saveSettings}
        showToast={showToast}
      />

      <button
        type="button"
        className="hatch-btn"
        onClick={() => void saveSettings(config)}
      >
        <HatchShadow
          size="sm"
          faceClassName="hatch-btn-face hatch-btn-face--primary"
        >
          Save settings
        </HatchShadow>
      </button>
    </div>
  );
}
