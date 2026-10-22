import { motion } from "motion/react";
import { LogIn, Plug, RefreshCw, Unplug } from "lucide-react";
import { useState } from "react";
import { bridge, type ProviderStatus } from "../../lib/bridge";
import { useOps } from "../../app/operations";
import { formatRelative, providerName, ROLE_LABEL } from "../../lib/format";

const ADAPTER: Record<ProviderStatus["adapter"], string> = { cli: "CLI", desktop_app: "приложение", mock: "демо (mock)", unavailable: "не подключён — ручной режим" };
import { ProviderStateLabel } from "../ui/StatusBadges";

interface Props {
  status: ProviderStatus;
  compact?: boolean;
  selected?: boolean;
  onSelect?: () => void;
}

const MARK: Record<ProviderStatus["provider"], string> = { codex: "Cx", claude: "Cl" };

/**
 * Карточка провайдера. Вход выполняется во внешнем официальном клиенте провайдера —
 * «Контур» не видит и не хранит пароли и токены (см. FRONTEND_HANDOFF.md → «Авторизация»).
 */
export function ProviderCard({ status, compact, selected, onSelect }: Props) {
  const { reportError } = useOps();
  const [pending, setPending] = useState(false);
  const busy = pending || status.state === "connecting";

  const act = async (fn: () => Promise<ProviderStatus>) => {
    setPending(true);
    try {
      await fn();
    } catch (e) {
      reportError(e, `${providerName(status.provider)}: ошибка`);
    } finally {
      setPending(false);
    }
  };

  const connect = () => act(() => bridge.connectProvider(status.provider));
  const disconnect = () => act(() => bridge.disconnectProvider(status.provider));

  const action =
    status.state === "connected" ? (
      <button className="btn btn--sm btn--ghost" onClick={disconnect} disabled={busy}>
        <Unplug /> Отключить
      </button>
    ) : status.state === "reauth_required" ? (
      <button className="btn btn--sm btn--primary" onClick={connect} disabled={busy}>
        <LogIn /> Войти снова
      </button>
    ) : status.state === "error" ? (
      <button className="btn btn--sm" onClick={connect} disabled={busy}>
        <RefreshCw /> Повторить
      </button>
    ) : (
      <button className="btn btn--sm" onClick={connect} disabled={busy}>
        <Plug /> {status.state === "connecting" ? "Подключение…" : "Подключить"}
      </button>
    );

  return (
    <div
      className={`provider${compact ? " provider--compact" : ""}`}
      data-state={status.state}
      data-selected={selected ?? false}
      role={onSelect ? "radio" : undefined}
      aria-checked={onSelect ? selected : undefined}
      tabIndex={onSelect ? 0 : undefined}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (onSelect && (e.key === "Enter" || e.key === " ") && e.target === e.currentTarget) {
          e.preventDefault();
          onSelect();
        }
      }}
    >
      {selected && <motion.span layoutId="provider-selected" className="provider__selected" />}
      <span className="provider__mark" aria-hidden>
        {MARK[status.provider]}
        {status.state === "connecting" && <span className="provider__ring" />}
      </span>
      <div className="provider__main">
        <div className="provider__name">{providerName(status.provider)}</div>
        <div className="provider__state">
          <ProviderStateLabel state={status.state} />
          {!compact && status.accountLabel && <span className="faint"> · {status.accountLabel}</span>}
        </div>
        {!compact && status.roles.length > 0 && (
          <div className="provider__roles">
            {status.roles.map((r) => (
              <span key={r} className="badge">
                {ROLE_LABEL[r]}
              </span>
            ))}
            <span className="faint">адаптер: {ADAPTER[status.adapter]}</span>
          </div>
        )}
        {!compact && status.errorMessage && <div className="provider__error">{status.errorMessage}</div>}
        {!compact && <div className="provider__checked faint">Проверено {formatRelative(status.lastCheckedAt)}</div>}
      </div>
      <div className="provider__action" onClick={(e) => e.stopPropagation()}>
        {action}
      </div>
    </div>
  );
}
