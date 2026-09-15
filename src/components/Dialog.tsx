import { useEffect, useId, type ReactNode } from "react";
import { HatchShadow } from "./HatchShadow";

export function Dialog({
  open,
  onClose,
  title,
  children,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
}) {
  const titleId = useId();

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="dialog-backdrop">
      <button
        type="button"
        className="dialog-scrim hatch-fill"
        aria-label="Close dialog"
        onClick={onClose}
      />
      <HatchShadow
        size="md"
        className="dialog-wrap"
        faceClassName="dialog-face"
      >
        <div
          role="dialog"
          aria-modal="true"
          aria-labelledby={titleId}
        >
          <div className="dialog-head">
            <h2 id={titleId}>{title}</h2>
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              onClick={onClose}
            >
              Esc
            </button>
          </div>
          <div className="dialog-body">{children}</div>
        </div>
      </HatchShadow>
    </div>
  );
}
