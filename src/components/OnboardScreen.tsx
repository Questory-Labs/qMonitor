import { BrandMark } from "./BrandMark";
import { HatchShadow } from "./HatchShadow";
import { QMark } from "./QMark";
import type { AppConfig } from "./Settings";

export function OnboardScreen({
  config,
  setConfig,
  loginPhase,
  showManualAuth,
  setShowManualAuth,
  callbackUrl,
  setCallbackUrl,
  testingUrl,
  saveAndTest,
  onStartLogin,
  onCancelLogin,
  onCompleteLogin,
}: {
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
  loginPhase: "idle" | "waiting";
  showManualAuth: boolean;
  setShowManualAuth: (v: boolean | ((p: boolean) => boolean)) => void;
  callbackUrl: string;
  setCallbackUrl: (v: string) => void;
  testingUrl: boolean;
  saveAndTest: () => void;
  onStartLogin: () => void;
  onCancelLogin: () => void;
  onCompleteLogin: () => void;
}) {
  return (
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
          <HatchShadow size="sm" faceClassName="panel onboard-panel">
            <div className="login-wait">
              <QMark variant="loading" />
              <h2 className="wait-title">Waiting for Questory…</h2>
              <p className="lede">
                Finish login in your browser. Listening on{" "}
                <code>127.0.0.1:58473</code> — you&apos;ll be signed in
                automatically.
              </p>
              <div className="actions actions--center">
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
                  <label className="label">
                    <span>Callback URL (with code=)</span>
                    <input
                      className="field"
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
          </HatchShadow>
        ) : (
          <HatchShadow size="sm" faceClassName="panel onboard-panel">
            <label className="label">
              <span>Questory URL</span>
              <input
                className="field"
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
                className="hatch-btn"
                onClick={saveAndTest}
                disabled={testingUrl}
              >
                <HatchShadow
                  size="sm"
                  faceClassName="hatch-btn-face hatch-btn-face--primary"
                >
                  {testingUrl ? "Testing…" : "Save & test"}
                </HatchShadow>
              </button>
              <button type="button" className="hatch-btn" onClick={onStartLogin}>
                <HatchShadow
                  size="sm"
                  faceClassName="hatch-btn-face hatch-btn-face--secondary"
                >
                  Log in with Questory
                </HatchShadow>
              </button>
            </div>
          </HatchShadow>
        )}
      </div>
    </div>
  );
}
