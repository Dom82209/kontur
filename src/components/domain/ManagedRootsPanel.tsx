import { FolderOpen, FolderPlus, Link, Plus, RefreshCw, Repeat } from "lucide-react";
import { useState } from "react";
import { useOps } from "../../app/operations";
import { useToast } from "../../app/toasts";
import { bridge, type ManagedRootKind, type ManagedRootState, type ManagedRootStatus, type ManagedRoots, type SelectableRootKind } from "../../lib/bridge";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";
import { ErrorState, SkeletonRows } from "../ui/States";
import { useWorkspaceTransition, workspaceKindLabel, workspaceLabel } from "./WorkspaceTransitionDialog";

const LABEL: Record<ManagedRootKind, { title: string; hint: string }> = {
  vault: { title: "Хранилище Obsidian", hint: "Структура предметов и недель создаётся внутри." },
  inbox: { title: "Папка «Входящие»", hint: "Всегда внутри хранилища: Предметы/00 Входящие." },
  backup: { title: "Резервные копии", hint: "Отдельная папка вне хранилища — для копий перед изменениями." },
  downloads: { title: "Наблюдаемые загрузки", hint: "Сюда браузер скачивает материалы и экспорт ЦУ. Наблюдение включается переключателем ниже." },
};

const STATE: Record<ManagedRootState, { label: string; tone: string }> = {
  ok: { label: "Доступна", tone: "badge--ok" },
  not_configured: { label: "Не выбрана", tone: "" },
  missing: { label: "Нет на диске", tone: "badge--warn" },
  unsafe: { label: "Небезопасна", tone: "badge--danger" },
};

const ORDER: ManagedRootKind[] = ["vault", "inbox", "backup", "downloads"];

/**
 * Управляемые каталоги. Папки выбираются только системным диалогом (backend);
 * абсолютные пути показываются для справки и не передаются обратно в команды.
 */
export function ManagedRootsPanel() {
  const toast = useToast();
  const { reportError } = useOps();
  const q = useBridgeQuery(() => bridge.getManagedRoots(), []);
  const [busy, setBusy] = useState<ManagedRootKind | "check" | null>(null);

  const apply = (roots: ManagedRoots) => q.setData(() => roots);

  const choose = async (kind: SelectableRootKind) => {
    setBusy(kind);
    try {
      const selection = await bridge.chooseManagedRoot(kind);
      apply(selection.roots);
      if (!selection.cancelled) toast.push({ tone: "ok", title: LABEL[kind].title, text: selection.roots[kind].displayPath ?? "Папка выбрана" });
    } catch (e) {
      reportError(e, `${LABEL[kind].title}: папка не принята`);
    } finally {
      setBusy(null);
    }
  };

  const prepareInbox = async () => {
    setBusy("inbox");
    try {
      apply(await bridge.prepareInboxRoot());
    } catch (e) {
      reportError(e, "Не удалось подготовить «Входящие»");
    } finally {
      setBusy(null);
    }
  };

  const check = async () => {
    setBusy("check");
    try {
      await q.reload();
    } finally {
      setBusy(null);
    }
  };

  const workspaces = useBridgeQuery(() => bridge.getWorkspaces(), []);
  const transition = useWorkspaceTransition(() => {
    void q.reload();
    void workspaces.reload();
  });
  const locked = busy !== null || transition.busy;

  if (q.error && !q.data) return <ErrorState message={q.error} onRetry={() => void q.reload()} />;
  if (!q.data) return <SkeletonRows count={4} />;
  const roots = q.data;

  return (
    <div className="roots">
      {transition.dialog}
      <div className="notice notice--info" data-testid="workspace-current">
        <strong>Текущее пространство: {workspaceKindLabel(roots.workspace?.kind)}</strong>
        <div className="field__hint mono">{roots.workspace?.id ?? "идентификатор ещё не создан"}</div>
        <p className="field__hint">Vault пространства нельзя заменить. Новый vault получает отдельную базу; прежний подключается только после проверки fingerprint. Каждый переход сначала показывает предпросмотр.</p>
        <div className="button-row">
          <button className="btn btn--sm" disabled={locked} onClick={() => void transition.start({ kind: "create_real" })}><Plus /> Создать рабочее пространство</button>
          <button className="btn btn--sm btn--ghost" disabled={locked} onClick={() => void transition.start({ kind: "connect_existing" })}><Link /> Подключить существующее</button>
          <button className="btn btn--sm btn--ghost" disabled={locked} onClick={() => void transition.start({ kind: "create_demo" })}>Новое демо</button>
        </div>
        {(workspaces.data?.workspaces.length ?? 0) > 1 && (
          <ul className="workspace-list">
            {workspaces.data!.workspaces
              .filter((item) => item.id !== roots.workspace?.id)
              .map((item) => (
                <li key={item.id}>
                  <span>{workspaceLabel(item)}</span>
                  <button className="btn btn--sm btn--ghost" disabled={locked} onClick={() => void transition.start({ kind: "activate", workspaceId: item.id })}>
                    <Repeat /> Переключиться…
                  </button>
                </li>
              ))}
          </ul>
        )}
      </div>
      {ORDER.map((kind) => (
        <RootRow
          key={kind}
          status={roots[kind]}
          busy={busy === kind}
          disabled={locked}
          onChoose={kind === "inbox" || kind === "vault" ? undefined : () => void choose(kind)}
          onPrepare={kind === "inbox" && roots.vault.state === "ok" && roots.inbox.state === "missing" ? () => void prepareInbox() : undefined}
        />
      ))}
      <div className="roots__foot">
        <span className={roots.importReady ? "faint" : "roots__blocked"}>
          {roots.importReady ? "Импорт во «Входящие» доступен." : "Импорт недоступен, пока не выбрано хранилище."}
        </span>
        <button className="btn btn--sm btn--ghost" onClick={() => void check()} disabled={locked}>
          <RefreshCw /> Проверить
        </button>
      </div>
    </div>
  );
}

function RootRow({ status, busy, disabled, onChoose, onPrepare }: { status: ManagedRootStatus; busy: boolean; disabled: boolean; onChoose?: () => void; onPrepare?: () => void }) {
  const label = LABEL[status.kind];
  // Отсутствующие «Входящие» — штатное состояние: папка создаётся при первом импорте.
  const state = status.kind === "inbox" && status.state === "missing" ? { label: "Будет создана", tone: "badge--info" } : STATE[status.state];
  const message = status.state === "ok" || status.message === "Не выбрана" ? label.hint : status.message ?? label.hint;
  return (
    <div className="roots__row" data-state={status.state}>
      <div className="roots__main">
        <div className="roots__title">
          {label.title} <span className={`badge ${state.tone}`}>{state.label}</span>
        </div>
        <div className="roots__path mono selectable">{status.displayPath ?? "—"}</div>
        <div className="field__hint">{message}</div>
      </div>
      {onChoose && (
        <button className="btn btn--sm" onClick={onChoose} disabled={disabled} aria-busy={busy}>
          <FolderOpen /> {status.state === "not_configured" ? "Выбрать…" : "Изменить…"}
        </button>
      )}
      {onPrepare && (
        <button className="btn btn--sm" onClick={onPrepare} disabled={disabled} aria-busy={busy}>
          <FolderPlus /> Создать
        </button>
      )}
    </div>
  );
}
