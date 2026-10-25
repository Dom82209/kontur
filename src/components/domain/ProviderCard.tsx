import { motion } from "motion/react";
import { ExternalLink, Plug, RefreshCw, Unplug } from "lucide-react";
import { useState } from "react";
import { AI_TASK_LABEL, bridge, type ProviderStatus } from "../../lib/bridge";
import { useOps } from "../../app/operations";
import { AVAILABILITY_LABEL, formatRelative, providerName } from "../../lib/format";
import { ProviderStateLabel } from "../ui/StatusBadges";

const ADAPTER: Record<ProviderStatus["adapter"], string> = { cli: "официальный CLI", desktop_app: "приложение", mock: "демо (mock)", manual_handoff: "ручная передача", unavailable: "CLI недоступен — ручная передача" };

interface Props {
  status: ProviderStatus;
  compact?: boolean;
  selected?: boolean;
  onSelect?: () => void;
}

const MARK: Record<ProviderStatus["provider"], string> = { codex: "Cx", claude: "Cl" };
const LOGIN: Record<ProviderStatus["provider"], string> = { codex: "codex login", claude: "claude auth login" };

/**
 * Карточка провайдера (этап 9). Вход выполняется только в официальном CLI или
 * приложении провайдера — «Контур» не видит и не хранит пароли и токены. Найденный
 * CLI не подключается сам: только по кнопке «Подключить CLI».
 */
export function ProviderCard({ status, compact, selected, onSelect }: Props) {
  const { reportError } = useOps();
  const [pending, setPending] = useState(false);
  const busy = pending || status.state === "connecting";

  const act = async (fn: () => Promise<unknown>) => {
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
  const check = () => act(() => bridge.checkProvider(status.provider));
  const openApp = () => act(() => bridge.openProviderApp(status.provider));
  const { cli } = status;

  const action = cli.enabled ? (
    <>
      <button className="btn btn--sm" onClick={check} disabled={busy}>
        <RefreshCw /> Проверить
      </button>
      <button className="btn btn--sm btn--ghost" onClick={disconnect} disabled={busy}>
        <Unplug /> Отключить CLI
      </button>
    </>
  ) : cli.detected ? (
    <button className="btn btn--sm" onClick={connect} disabled={busy}>
      <Plug /> {status.state === "connecting" ? "Проверка…" : "Подключить CLI"}
    </button>
  ) : (
    <button className="btn btn--sm" onClick={check} disabled={busy}>
      <RefreshCw /> Искать CLI
    </button>
  );

  return (
    <div
      className={`provider${compact ? " provider--compact" : ""}`}
      data-state={status.state}
      data-availability={status.availability}
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
          {status.availability === "manual" ? <span>{AVAILABILITY_LABEL.manual}</span> : <ProviderStateLabel state={status.state} />}
          {!compact && status.accountLabel && <span className="faint"> · {status.accountLabel}</span>}
        </div>
        {!compact && (
          <div className="provider__roles">
            {status.tasks.map((t) => (
              <span key={t} className="badge">
                {AI_TASK_LABEL[t]}
              </span>
            ))}
            <span className="faint">способ: {ADAPTER[status.adapter]}</span>
          </div>
        )}
        {!compact && (
          <div className="provider__cli faint">
            {cli.detected
              ? `CLI найден: ${cli.executable ?? "—"}${cli.version ? ` · ${cli.version}` : ""}${cli.enabled ? "" : " · не подключён"}`
              : "Официальный CLI не найден — шаги выполняются ручной передачей"}
            {cli.loggedIn === false && !status.errorMessage && ` · нужен вход: выполните «${LOGIN[status.provider]}» в Терминале`}
          </div>
        )}
        {!compact && status.errorMessage && <div className="provider__error">{status.errorMessage}</div>}
        {!compact && cli.checkedAt && <div className="provider__checked faint">Проверено {formatRelative(cli.checkedAt)}</div>}
      </div>
      <div className="provider__action" onClick={(e) => e.stopPropagation()}>
        {action}
        {!compact && (
          <button className="btn btn--sm btn--ghost" onClick={openApp} disabled={busy} title={status.handoff.appInstalled ? "Официальное приложение" : status.handoff.webUrl}>
            <ExternalLink /> {status.handoff.appInstalled ? "Приложение" : "Сайт"}
          </button>
        )}
      </div>
    </div>
  );
}
