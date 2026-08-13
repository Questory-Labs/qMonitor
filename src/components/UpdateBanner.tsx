import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";

export interface PendingUpdate {
  tag: string;
  version: string;
  htmlUrl: string;
}

export function UpdateBanner() {
  const [pending, setPending] = useState<PendingUpdate | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    invoke<PendingUpdate | null>("check_for_updates", { force: false })
      .then((p) => {
        if (!cancelled) setPending(p);
      })
      .catch(() => {
        /* offline / first-run — banner stays hidden */
      });

    listen<PendingUpdate>("qmonitor://update-available", (event) => {
      setPending(event.payload);
    }).then((fn) => unsubs.push(fn));

    listen("qmonitor://update-clear", () => {
      setPending(null);
    }).then((fn) => unsubs.push(fn));

    return () => {
      cancelled = true;
      unsubs.forEach((u) => u());
    };
  }, []);

  if (!pending) return null;

  async function openRelease() {
    if (!pending) return;
    setBusy(true);
    try {
      await invoke("open_release_url", { url: pending.htmlUrl });
    } finally {
      setBusy(false);
    }
  }

  async function dismiss() {
    setBusy(true);
    try {
      await invoke("dismiss_update");
      setPending(null);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="update-banner" role="status">
      <p>
        Update available: <strong>{pending.version}</strong>
      </p>
      <div className="update-banner-actions">
        <button
          type="button"
          className="btn btn-primary btn-sm"
          disabled={busy}
          onClick={() => void openRelease()}
        >
          Open release
        </button>
        <button
          type="button"
          className="btn btn-ghost btn-sm"
          disabled={busy}
          onClick={() => void dismiss()}
        >
          Dismiss
        </button>
      </div>
    </div>
  );
}
