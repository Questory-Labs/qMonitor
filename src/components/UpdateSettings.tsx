import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { PendingUpdate } from "./UpdateBanner";

export type UpdateChannel = "stable" | "canary";

export function UpdateSettings({
  channel,
  onChannelChange,
  showToast,
}: {
  channel: UpdateChannel;
  onChannelChange: (channel: UpdateChannel) => Promise<void>;
  showToast: (text: string, isError?: boolean) => void;
}) {
  const [version, setVersion] = useState<string>("");
  const [checking, setChecking] = useState(false);

  useEffect(() => {
    invoke<string>("get_app_version")
      .then(setVersion)
      .catch(() => setVersion(""));
  }, []);

  async function checkNow() {
    setChecking(true);
    try {
      const pending = await invoke<PendingUpdate | null>("check_for_updates", {
        force: true,
      });
      if (pending) {
        showToast(`Update available: ${pending.version}`);
      } else {
        showToast("You're up to date");
      }
    } catch (e) {
      showToast(String(e), true);
    } finally {
      setChecking(false);
    }
  }

  return (
    <section className="settings-card">
      <h2 className="section-label">Updates</h2>
      <div className="setting-row">
        <div className="setting-row-text">
          <span className="setting-row-label">Version</span>
          <span className="setting-row-hint">
            {version || "…"} · checks GitHub once per day
          </span>
        </div>
        <button
          type="button"
          className="btn btn-secondary btn-sm"
          disabled={checking}
          onClick={() => void checkNow()}
        >
          {checking ? "Checking…" : "Check now"}
        </button>
      </div>
      <label className="field">
        <span>Release channel</span>
        <select
          value={channel}
          onChange={(e) =>
            void onChannelChange(e.target.value as UpdateChannel)
          }
        >
          <option value="stable">Stable (Latest)</option>
          <option value="canary">Canary (prerelease)</option>
        </select>
      </label>
    </section>
  );
}
