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
    <>
      <h2 className="section-label">Updates</h2>
      <p className="meta">
        Installed version {version || "…"} · checks GitHub once per day
      </p>
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
      <div className="actions">
        <button
          type="button"
          className="btn btn-secondary"
          disabled={checking}
          onClick={() => void checkNow()}
        >
          {checking ? "Checking…" : "Check now"}
        </button>
      </div>
    </>
  );
}
