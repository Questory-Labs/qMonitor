import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { useState, type ReactNode } from "react";
import { invokeTimeout } from "../invoke-timeout";
import { HatchShadow } from "./HatchShadow";
import type { AppConfig, LogLevel } from "./Settings";

function Card({ children }: { children: ReactNode }) {
  return (
    <HatchShadow size="sm" faceClassName="panel settings-face">
      {children}
    </HatchShadow>
  );
}

function FieldActions({ children }: { children: ReactNode }) {
  return <div className="field-actions">{children}</div>;
}

export function AdvancedSettings({
  config,
  setConfig,
  saveSettings,
  showToast,
}: {
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
  saveSettings: (next: AppConfig) => Promise<void>;
  showToast: (text: string, isError?: boolean) => void;
}) {
  async function openLogs() {
    try {
      const path = await invokeTimeout<string>("open_log_dir", 8000);
      showToast(`Opened ${path}`);
    } catch (e) {
      showToast(String(e), true);
    }
  }

  async function openDb() {
    try {
      const path = await invokeTimeout<string>("open_db", 8000);
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
      const selected = await openFileDialog({
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

  const [expanded, setExpanded] = useState(
    () =>
      import.meta.env.DEV &&
      new URLSearchParams(window.location.search).has("advanced"),
  );

  return (
    <details
      className="settings-advanced"
      open={expanded}
      onToggle={(e) => setExpanded(e.currentTarget.open)}
    >
      <summary className="hatch-shadow hatch-shadow--sm settings-advanced-toggle">
        <span className="hatch-cast" aria-hidden />
        <span className="hatch-face panel settings-advanced-face">
          Advanced
        </span>
      </summary>

      <div className="settings-advanced-body">
        <Card>
          <h2 className="section-label">Catalog</h2>
          <label className="label">
            <span>Catalog path</span>
            <input
              className="field field-mono"
              value={config.catalogPath ?? ""}
              onChange={(e) =>
                setConfig({ ...config, catalogPath: e.target.value })
              }
              placeholder="catalogs/games.example.json"
              aria-label="Catalog path"
              spellCheck={false}
            />
            <FieldActions>
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={() => void browseCatalog()}
              >
                Browse
              </button>
            </FieldActions>
          </label>
          <label className="label">
            <span>Detectable catalog URL</span>
            <input
              className="field field-mono"
              value={config.detectableUrl ?? ""}
              onChange={(e) =>
                setConfig({ ...config, detectableUrl: e.target.value })
              }
              placeholder="https://discord.com/api/v10/applications/detectable"
              title={config.detectableUrl || undefined}
              spellCheck={false}
            />
          </label>
        </Card>

        <Card>
          <h2 className="section-label">Logging</h2>
          <div className="setting-row">
            <span className="setting-row-label">Log level</span>
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              onClick={() => void openLogs()}
            >
              Open folder
            </button>
          </div>
          <label className="label">
            <span className="sr-only">Log level</span>
            <select
              className="field"
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
            <p className="setting-row-hint setting-row-hint--wrap">
              Off by default · 3 days · 5 MB cap
            </p>
          </label>
        </Card>

        <Card>
          <h2 className="section-label">Database</h2>
          <label className="label">
            <span>Path</span>
            <input
              className="field field-mono"
              value={config.dbPath ?? ""}
              onChange={(e) =>
                setConfig({ ...config, dbPath: e.target.value })
              }
              placeholder="Default: config dir / qmonitor.db"
              aria-label="Database path"
              spellCheck={false}
            />
            <FieldActions>
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={() => void openDb()}
              >
                Open DB
              </button>
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={() => void browseDb()}
              >
                Browse
              </button>
            </FieldActions>
          </label>
        </Card>

        <Card>
          <h2 className="section-label">Dev</h2>
          <label className="label">
            <span>Access token</span>
            <input
              className="field field-mono"
              type="password"
              value={config.devAccessToken ?? ""}
              onChange={(e) =>
                setConfig({
                  ...config,
                  devAccessToken: e.target.value,
                })
              }
              autoComplete="off"
            />
          </label>
        </Card>
      </div>
    </details>
  );
}
