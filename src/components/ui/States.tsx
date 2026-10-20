import { CircleAlert, RefreshCw } from "lucide-react";
import type { CSSProperties, ReactNode } from "react";

export function EmptyState({ icon, title, text, action }: { icon: ReactNode; title: string; text?: string; action?: ReactNode }) {
  return (
    <div className="empty">
      <div className="empty__icon">{icon}</div>
      <div className="empty__title">{title}</div>
      {text && <p className="empty__text">{text}</p>}
      {action}
    </div>
  );
}

export function ErrorState({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="notice notice--danger" role="alert">
      <CircleAlert />
      <div>
        <strong>Не удалось загрузить данные.</strong> {message}
      </div>
      <button className="btn btn--sm notice__action" onClick={onRetry}>
        <RefreshCw /> Повторить
      </button>
    </div>
  );
}

export function Skeleton({ h = 14, w = "100%", style }: { h?: number; w?: number | string; style?: CSSProperties }) {
  return <div className="skeleton" aria-hidden style={{ height: h, width: w, ...style }} />;
}

export function SkeletonRows({ count = 4 }: { count?: number }) {
  return (
    <div style={{ display: "grid", gap: 10, padding: "6px 10px" }} aria-busy="true" aria-label="Загрузка">
      {Array.from({ length: count }, (_, i) => (
        <div key={i} style={{ display: "flex", gap: 12, alignItems: "center" }}>
          <Skeleton h={30} w={30} />
          <div style={{ flex: 1, display: "grid", gap: 6 }}>
            <Skeleton h={11} w={`${70 - i * 9}%`} />
            <Skeleton h={9} w={`${40 - i * 4}%`} />
          </div>
        </div>
      ))}
    </div>
  );
}
