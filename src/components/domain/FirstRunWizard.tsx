import { FolderOpen, FlaskConical, Link, ShieldCheck } from "lucide-react";
import { useState } from "react";
import { useOps } from "../../app/operations";
import { bridge, type ManagedRoots } from "../../lib/bridge";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";
import { Dialog } from "../ui/Dialog";
import { useWorkspaceTransition } from "./WorkspaceTransitionDialog";

/** Первый запуск: корни выбираются native-диалогом, структура создаётся только после preview. */
export function FirstRunWizard() {
  const query = useBridgeQuery(() => bridge.getManagedRoots(), []);
  const { reportError } = useOps();
  const [dismissed, setDismissed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState(false);
  const roots = query.data;
  const open = !dismissed && !!roots && roots.inbox.state !== "ok";

  const run = async (action: () => Promise<ManagedRoots>, title: string, reload = false) => {
    setBusy(true);
    try {
      const next = await action();
      query.setData(() => next);
      if (reload) window.location.reload();
    } catch (error) {
      reportError(error, title);
    } finally {
      setBusy(false);
    }
  };
  const transition = useWorkspaceTransition(() => void query.reload());
  const locked = busy || transition.busy;

  return (
    <>
    {transition.dialog}
    <Dialog
      open={open && !transition.dialog}
      onClose={() => setDismissed(true)}
      title="Настроим безопасное рабочее пространство"
      description="«Контур» не выбирает Desktop, домашнюю папку или корень диска автоматически. Все папки проверяет backend."
      wide
      footer={
        <>
          <button className="btn btn--ghost" disabled={locked} onClick={() => void transition.start({ kind: "create_demo" })}>
            <FlaskConical /> Создать демо
          </button>
          <button className="btn btn--ghost" disabled={locked} onClick={() => void transition.start({ kind: "connect_existing" })}>
            <Link /> Подключить существующее
          </button>
          {preview ? (
            <button
              className="btn btn--primary"
              disabled={locked || roots?.vault.state !== "ok"}
              onClick={() => void run(() => bridge.prepareInboxRoot(), "Структура не создана")}
            >
              <ShieldCheck /> Подтвердить и создать
            </button>
          ) : (
            <button className="btn btn--primary" disabled={locked || roots?.vault.state !== "ok"} onClick={() => setPreview(true)}>
              Предпросмотр структуры
            </button>
          )}
        </>
      }
    >
      <ol className="onboarding-steps">
        <li>
          <div><strong>Учебное хранилище и Obsidian vault</strong><p className="field__hint">Каждый vault получает отдельную базу. Данные demo и других vault не смешиваются.</p></div>
          <button className="btn btn--sm" disabled={locked} onClick={() => void transition.start({ kind: "create_real" })}><FolderOpen /> Создать рабочее пространство…</button>
          <span className="mono faint">{roots?.vault.displayPath ?? "не выбрано"}</span>
        </li>
      </ol>
      <div className="notice notice--warn">Vault активного пространства нельзя тихо заменить. Для другого vault создайте новое пространство или повторно подключите ранее зарегистрированное.</div>
      {preview && (
        <div className="notice notice--info">
          <strong>Будет создано только после подтверждения:</strong>
          <div className="mono">Предметы/00 Входящие</div>
          <div className="field__hint">Папки предметов, недель и ДЗ появятся позднее — при подтверждённой раскладке. Существующие файлы не перезаписываются.</div>
        </div>
      )}
      <p className="field__hint">Демо создаётся в каталоге данных «Контура» и не затрагивает реальные учебные материалы.</p>
    </Dialog>
    </>
  );
}
