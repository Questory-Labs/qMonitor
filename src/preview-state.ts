import type { AppConfig, AuthState } from "./components/Settings";

export function isTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Vite-only mock so `pnpm dev` can show the UI without Tauri. */
export function isBrowserPreview() {
  return import.meta.env.DEV && !isTauriRuntime();
}

export const PREVIEW_CONFIG: AppConfig = {
  baseUrl: "https://app.questorylabs.com",
  pollIntervalSecs: 3,
  retentionAckedDays: 7,
  minPushDurationMins: 1,
  pushFromListOnly: false,
  startAtLogin: true,
  minimizeToTray: true,
  closeToTray: true,
  updateChannel: "stable",
  logLevel: "error",
  catalogPath: "catalogs/games.example.json",
  detectableUrl: "https://discord.com/api/v10/applications/detectable",
};

export const PREVIEW_AUTH: AuthState = {
  baseUrl: "https://app.questorylabs.com",
  webhookUrl: "https://app.questorylabs.com/webhooks/qmonitor",
  hasAccessToken: true,
  hasSessionToken: true,
};

export const PREVIEW_GAMES = [
  {
    id: "steam:1172470",
    title: "Apex Legends",
    steamAppId: 1172470,
    source: "steam",
    trackingEnabled: true,
  },
  {
    id: "steam:730",
    title: "Counter-Strike 2",
    steamAppId: 730,
    source: "steam",
    trackingEnabled: true,
  },
  {
    id: "manual:crimson",
    title: "Crimson Desert Enhanced",
    source: "manual",
    trackingEnabled: false,
  },
];

export const PREVIEW_HOME = {
  sync: {
    tursoOk: true,
    pendingCount: 0,
    webhookConfigured: true,
    loopAlive: true,
    lastTickAt: new Date().toISOString(),
  },
  history: [
    {
      id: "s1",
      identityId: "steam:1172470",
      title: "Apex Legends",
      steamAppId: 1172470,
      source: "steam",
      startedAt: "2026-09-15T13:31:47.000Z",
      endedAt: "2026-09-15T13:32:35.000Z",
      durationSecs: 48,
      pushStatus: "skipped" as const,
      lastError: "too_short",
    },
    {
      id: "s2",
      identityId: "manual:crimson",
      title: "Crimson Desert Enhanced",
      source: "manual",
      startedAt: "2026-09-14T16:01:00.000Z",
      endedAt: "2026-09-14T17:28:34.000Z",
      durationSecs: 5220,
      pushStatus: "synced" as const,
    },
    {
      id: "s3",
      identityId: "steam:730",
      title: "Counter-Strike 2",
      steamAppId: 730,
      source: "steam",
      startedAt: "2026-09-14T14:41:00.000Z",
      endedAt: "2026-09-14T15:56:34.000Z",
      durationSecs: 4500,
      pushStatus: "synced" as const,
    },
    {
      id: "s4",
      identityId: "manual:crimson",
      title: "Crimson Desert Enhanced",
      source: "manual",
      startedAt: "2026-09-14T11:13:00.000Z",
      endedAt: "2026-09-14T12:27:56.000Z",
      durationSecs: 4440,
      pushStatus: "synced" as const,
    },
    {
      id: "s5",
      identityId: "manual:crimson",
      title: "Crimson Desert Enhanced",
      source: "manual",
      startedAt: "2026-09-14T09:32:00.000Z",
      endedAt: "2026-09-14T10:10:07.000Z",
      durationSecs: 2280,
      pushStatus: "synced" as const,
    },
  ],
  pendingDetections: [
    {
      processName: "Hades.exe",
      exePath: "D:\\Games\\Hades\\Hades.exe",
      fingerprint: "preview-hades",
      suggestedTitle: "Hades",
    },
  ],
};

export function previewOnboarded() {
  return !new URLSearchParams(window.location.search).has("onboard");
}
