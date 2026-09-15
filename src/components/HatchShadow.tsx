import type { ReactNode } from "react";

export type HatchShadowSize = "sm" | "md" | "lg";

/**
 * Drawn drop-shadow: a hatched slab sits behind an opaque face.
 * Matches @questorylabs/ui HatchShadow.
 */
export function HatchShadow({
  children,
  size = "md",
  className,
  faceClassName,
}: {
  children: ReactNode;
  size?: HatchShadowSize;
  className?: string;
  faceClassName?: string;
}) {
  return (
    <div
      className={["hatch-shadow", `hatch-shadow--${size}`, className]
        .filter(Boolean)
        .join(" ")}
    >
      <span className="hatch-cast" aria-hidden />
      <div
        className={["hatch-face", faceClassName].filter(Boolean).join(" ")}
      >
        {children}
      </div>
    </div>
  );
}
