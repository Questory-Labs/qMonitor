import { useId, type CSSProperties } from "react";
import "./QMark.css";

/* ------------------------------------------------------------------ */
/*  Geometry for the Questory Q-mark                                   */
/*                                                                     */
/*  Thick near-complete circle (--ink) with an orange diagonal slash   */
/*  (--warm) through the gap — the "tail" of the Q.                    */
/*                                                                     */
/*  Animation flow (see QMark.css):                                    */
/*    Phase 1  GEOMETRY  — dot, crosshairs, guide circle, ticks        */
/*    Phase 2  LOGO      — arc brush draws ring, tail slashes in       */
/*    Phase 3  GLOW      — scaffolding dissolves, Q glows              */
/*    Phase 4  FADE      — Q fades out, blank pause, restart           */
/* ------------------------------------------------------------------ */
const S = 48;
const CX = 24;
const CY = 24;
const R = 16;
const CIRC = 2 * Math.PI * R;
// Rotate the mask start so the sweep begins at ~2 o'clock
const SWEEP_START = -290;
const EXT = 2;

const TAIL = { x1: 20, y1: 26, x2: 45, y2: 45 };
const TAIL_LEN = Math.sqrt(
  (TAIL.x2 - TAIL.x1) ** 2 + (TAIL.y2 - TAIL.y1) ** 2,
);

/** 8 perimeter tick marks (every 45°). */
const TICKS = Array.from({ length: 8 }, (_, i) => {
  const a = (i * 45 - 90) * (Math.PI / 180);
  return {
    x1: CX + Math.cos(a) * (R - 2),
    y1: CY + Math.sin(a) * (R - 2),
    x2: CX + Math.cos(a) * (R + 2.5),
    y2: CY + Math.sin(a) * (R + 2.5),
  };
});

export function QMark({ variant }: { variant: "loading" | "error" }) {
  const uid = useId().replace(/:/g, "");
  const ringMaskId = `q-ring-mask-${uid}`;
  const tailMaskId = `q-tail-mask-${uid}`;
  const isError = variant === "error";
  const ringColor = isError ? "var(--danger)" : "var(--ink)";
  const tailColor = isError ? "var(--danger)" : "var(--warm)";
  const scaffold = isError ? "var(--danger)" : "var(--faint)";
  const glowColor = isError ? "var(--danger)" : "var(--warm)";
  const dh = R * 0.65;

  const rootStyle = {
    width: S,
    height: S,
    "--qc-glow": glowColor,
    "--qc-circ": String(CIRC),
    "--qc-tail-len": String(TAIL_LEN),
  } as CSSProperties;

  return (
    <div
      className={`qc-root${isError ? " qc-anim-shake" : ""}`}
      style={rootStyle}
    >
      <svg
        viewBox={`0 0 ${S} ${S}`}
        width={S}
        height={S}
        fill="none"
        overflow="visible"
        aria-hidden
        className="qc-anim-glow"
      >
        <g className="qc-scaffold">
          <circle
            cx={CX}
            cy={CY}
            r={2}
            fill={scaffold}
            className="qc-anim-dot"
            style={{ transformOrigin: `${CX}px ${CY}px` }}
          />

          <line
            x1={CX}
            y1={CY - R - EXT}
            x2={CX}
            y2={CY + R + EXT}
            stroke={scaffold}
            strokeWidth={0.5}
            className="qc-anim-cross-v"
            style={{ transformOrigin: `${CX}px ${CY}px` }}
          />

          <line
            x1={CX - R - EXT}
            y1={CY}
            x2={CX + R + EXT}
            y2={CY}
            stroke={scaffold}
            strokeWidth={0.5}
            className="qc-anim-cross-h"
            style={{ transformOrigin: `${CX}px ${CY}px` }}
          />

          <g
            className="qc-anim-diag"
            style={{ transformOrigin: `${CX}px ${CY}px` }}
          >
            <line
              x1={CX}
              y1={CY - R - EXT}
              x2={CX}
              y2={CY + R + EXT}
              stroke={scaffold}
              strokeWidth={0.5}
            />
            <line
              x1={CX - R - EXT}
              y1={CY}
              x2={CX + R + EXT}
              y2={CY}
              stroke={scaffold}
              strokeWidth={0.5}
            />
          </g>

          <rect
            x={CX - dh}
            y={CY - dh}
            width={dh * 2}
            height={dh * 2}
            stroke={scaffold}
            strokeWidth={0.6}
            fill="none"
            rx={1}
            className="qc-anim-diamond"
            style={{ transformOrigin: `${CX}px ${CY}px` }}
          />

          <circle
            cx={CX}
            cy={CY}
            r={R}
            stroke={scaffold}
            strokeWidth={0.5}
            fill="none"
            strokeDasharray="2 4"
            className="qc-anim-guide"
          />

          {TICKS.map((t, i) => (
            <line
              key={i}
              x1={t.x1}
              y1={t.y1}
              x2={t.x2}
              y2={t.y2}
              stroke={scaffold}
              strokeWidth={0.7}
              className="qc-anim-tick"
              style={{ transformOrigin: `${CX}px ${CY}px` }}
            />
          ))}
        </g>

        <mask id={ringMaskId}>
          <circle
            cx={CX}
            cy={CY}
            r={R}
            stroke="white"
            strokeWidth={14}
            fill="none"
            strokeLinecap="butt"
            strokeDasharray={CIRC}
            strokeDashoffset={CIRC}
            className="qc-anim-arc"
            style={{
              transformOrigin: `${CX}px ${CY}px`,
              transform: `rotate(${SWEEP_START}deg)`,
            }}
          />
        </mask>

        <mask id={tailMaskId}>
          <line
            x1={TAIL.x1}
            y1={TAIL.y1}
            x2={TAIL.x2}
            y2={TAIL.y2}
            stroke="white"
            strokeWidth={16}
            strokeLinecap="round"
            strokeDasharray={TAIL_LEN}
            strokeDashoffset={TAIL_LEN}
            className="qc-anim-tail"
          />
        </mask>

        <g mask={`url(#${ringMaskId})`}>
          <g transform="translate(5.17, 5.17) scale(0.10318)">
            <path
              fillRule="evenodd"
              fill={ringColor}
              d="m323.285 297.021-36.426-39.312c15.271-21.152 24.275-47.127 24.275-75.209 0-71.043-57.591-128.634-128.634-128.634S53.866 111.457 53.866 182.5 111.457 311.134 182.5 311.134c7.314 0 14.478-.629 21.458-1.802l39.737 44.054c-19.121 6.85-39.718 10.6-61.195 10.6-100.232 0-181.486-81.254-181.486-181.486S82.268 1.014 182.5 1.014 363.986 82.268 363.986 182.5c0 43.426-15.261 83.284-40.701 114.521Z"
            />
          </g>
        </g>

        <g mask={`url(#${tailMaskId})`}>
          <g transform="translate(5.17, 5.17) scale(0.10318)">
            <path
              fillRule="evenodd"
              fill={tailColor}
              d="M185 245h64.1L351 353.1l-66.819 1.857z"
            />
          </g>
        </g>

        {isError ? (
          <g
            className="qc-anim-bang"
            style={{ transformOrigin: `${CX}px ${CY}px` }}
          >
            <line
              x1={CX}
              y1={CY - 5}
              x2={CX}
              y2={CY + 1}
              stroke="var(--ink)"
              strokeWidth={2.5}
              strokeLinecap="round"
            />
            <circle cx={CX} cy={CY + 5} r={1.4} fill="var(--ink)" />
          </g>
        ) : null}
      </svg>
    </div>
  );
}
