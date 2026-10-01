import type { ReactNode } from "react";
import { HatchShadow } from "./HatchShadow";

export function EmptyFace({
  title,
  description,
}: {
  title: ReactNode;
  description?: ReactNode;
}) {
  return (
    <HatchShadow size="sm" faceClassName="panel empty-face">
      <p className="empty-title">{title}</p>
      {description ? <p className="empty-desc">{description}</p> : null}
    </HatchShadow>
  );
}
