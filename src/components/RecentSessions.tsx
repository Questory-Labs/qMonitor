import { useState } from "react";
import { Dialog } from "./Dialog";
import { EmptyFace } from "./EmptyFace";

export type RecentSession = {
  id: string;
  identityId: string;
  title: string;
  durationSecs?: number;
  endedAt?: string;
  pushStatus: "active" | "pending" | "synced" | "failed" | "skipped";
  lastError?: string;
};

function formatDuration(secs?: number) {
  if (secs == null) return "—";
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0) return m > 0 ? `${h}h ${m}m` : `${h}h`;
  if (m > 0) return s > 0 ? `${m}m ${s}s` : `${m}m`;
  return `${s}s`;
}

function formatWhen(iso?: string) {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const time = d.toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return time;
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (d.toDateString() === yesterday.toDateString()) return `Yesterday ${time}`;
  return `${d.toLocaleDateString(undefined, { day: "numeric", month: "short" })} ${time}`;
}

function statusReason(session: RecentSession) {
  switch (session.pushStatus) {
    case "skipped":
      if (session.lastError === "too_short") return "Too short";
      if (session.lastError === "not_on_list") return "Not on list";
      return "Skipped";
    case "failed":
      return "Failed";
    case "pending":
      return "Pending";
    default:
      return null;
  }
}

function canAct(session: RecentSession) {
  return (
    session.pushStatus === "skipped" ||
    session.pushStatus === "failed" ||
    session.pushStatus === "pending"
  );
}

function actionCopy(session: RecentSession) {
  switch (session.pushStatus) {
    case "skipped":
      if (session.lastError === "too_short") {
        return "Under the minimum session length, so it stayed local.";
      }
      if (session.lastError === "not_on_list") {
        return "This title is not on the auto-sync list.";
      }
      return "This session was not synced.";
    case "failed":
      return session.lastError || "Sync failed. You can try again.";
    case "pending":
      return "Waiting to sync. Force it now if it is stuck.";
    default:
      return "";
  }
}

function SessionLine({ session }: { session: RecentSession }) {
  const reason = statusReason(session);
  const when = formatWhen(session.endedAt);
  return (
    <>
      <strong>{session.title}</strong>
      <span className="history-duration">
        {formatDuration(session.durationSecs)}
      </span>
      <div className="meta">
        {reason ? <span className="history-reason">{reason}</span> : null}
        {reason && when ? " · " : null}
        {when || (reason ? null : "—")}
      </div>
    </>
  );
}

export function RecentSessions({
  history,
  onSync,
  onIgnore,
}: {
  history: RecentSession[];
  onSync: (session: RecentSession) => Promise<void>;
  onIgnore: (session: RecentSession) => Promise<void>;
}) {
  const [selected, setSelected] = useState<RecentSession | null>(null);
  const [busy, setBusy] = useState<"sync" | "ignore" | null>(null);

  async function run(kind: "sync" | "ignore") {
    if (!selected) return;
    setBusy(kind);
    try {
      if (kind === "sync") await onSync(selected);
      else await onIgnore(selected);
      setSelected(null);
    } finally {
      setBusy(null);
    }
  }

  if (history.length === 0) {
    return <EmptyFace title="No sessions yet" />;
  }

  return (
    <>
      <div className="panel-outline">
        <ul className="session-list">
          {history.map((session) => {
            const actionable = canAct(session);
            const className = [
              "row-item",
              "history-row",
              session.pushStatus === "skipped" ? "is-skipped" : "",
              session.pushStatus === "failed" ? "is-failed" : "",
              session.pushStatus === "pending" ? "is-pending" : "",
              actionable ? "history-row--action" : "",
            ]
              .filter(Boolean)
              .join(" ");

            return (
              <li key={session.id}>
                {actionable ? (
                  <button
                    type="button"
                    className={className}
                    onClick={() => setSelected(session)}
                    aria-label={`${session.title}, ${formatDuration(session.durationSecs)}, ${statusReason(session) ?? ""}. Open actions`}
                  >
                    <SessionLine session={session} />
                  </button>
                ) : (
                  <div className={className}>
                    <SessionLine session={session} />
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      </div>

      <Dialog
        open={selected != null}
        onClose={() => {
          if (!busy) setSelected(null);
        }}
        title={selected?.title ?? ""}
      >
        {selected ? (
          <>
            <p className="history-dialog-kicker">
              {formatDuration(selected.durationSecs)}
              {selected.endedAt ? ` · ${formatWhen(selected.endedAt)}` : ""}
            </p>
            <p className="lede">{actionCopy(selected)}</p>
            <div className="actions dialog-actions">
              <button
                type="button"
                className="btn btn-secondary"
                disabled={busy != null}
                onClick={() => void run("ignore")}
              >
                {busy === "ignore" ? "Working…" : "Don't track"}
              </button>
              <button
                type="button"
                className="btn btn-primary"
                disabled={busy != null}
                onClick={() => void run("sync")}
              >
                {busy === "sync" ? "Syncing…" : "Sync"}
              </button>
            </div>
          </>
        ) : null}
      </Dialog>
    </>
  );
}
