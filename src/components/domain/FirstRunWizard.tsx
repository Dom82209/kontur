import { FolderOpen, FlaskConical, ShieldCheck } from "lucide-react";
import { useState } from "react";
import { useOps } from "../../app/operations";
import { bridge, type ManagedRoots, type SelectableRootKind } from "../../lib/bridge";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";
import { Dialog } from "../ui/Dialog";

/** Первый запуск: корни выбираются native-диалогом, структура создаётся только после preview. */
export function FirstRunWizard() {
  const query = useBridgeQuery(() => bridge.getManagedRoots(), []);
  const { reportError } = useOps();
  const [dismissed, setDismissed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState(false);
  const roots = query.data;
  const open = !dismissed && !!roots && !roots.importReady;

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
  const choose = async (kind: SelectableRootKind) => {
    setBusy(true);
    try {
      const result = await bridge.chooseManagedRoot(kind);
      query.setData(() => result.roots);
    } catch (error) {
      reportError(error, "Папка не выбрана");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onClose={() => setDismissed(true)}
      title="Настроим безопасное рабочее пространство"
      description="«Контур» не выбирает Desktop, домашнюю папку или корень диска автоматически. Все папки проверяет backend."
      wide
      footer={
        <>
          <button className="btn btn--ghost" disabled={busy} onClick={() => void run(() => bridge.createDemoWorkspace(), "Демо-пространство не создано", true)}>
            <FlaskConical /> Создать демо
          </button>
          {preview ? (
            <button
              className="btn btn--primary"
              disabled={busy || roots?.vault.state !== "ok"}
              onClick={() => void run(() => bridge.prepareInboxRoot(), "Структура не создана")}
            >
              <ShieldCheck /> Подтвердить и создать
            </button>
          ) : (
            <button className="btn btn--primary" disabled={busy || roots?.vault.state !== "ok"} onClick={() => setPreview(true)}>
              Предпросмотр структуры
            </button>
          )}
        </>
      }
    >
      <ol className="onboarding-steps">
        <li>
          <div><strong>Учебное хранилище и Obsidian vault</strong><p className="field__hint">Единый безопасный корень сохраняет относительные Obsidian-ссылки. Можно выбрать уже созданный vault.</p></div>
          <button className="btn btn--sm" disabled={busy} onClick={() => void choose("vault")}><FolderOpen /> Выбрать…</button>
          <span className="mono faint">{roots?.vault.displayPath ?? "не выбрано"}</span>
        </li>
        <li>
          <div><strong>Папка загрузок</strong><p className="field__hint">Выбирается отдельно; наблюдение останется выключенным до вашего действия.</p></div>
          <button className="btn btn--sm" disabled={busy} onClick={() => void choose("downloads")}><FolderOpen /> Выбрать…</button>
          <span className="mono faint">{roots?.downloads.displayPath ?? "необязательно"}</span>
        </li>
        <li>
          <div><strong>Резервные копии</strong><p className="field__hint">Рекомендуется отдельная папка вне учебного хранилища.</p></div>
          <button className="btn btn--sm" disabled={busy} onClick={() => void choose("backup")}><FolderOpen /> Выбрать…</button>
          <span className="mono faint">{roots?.backup.displayPath ?? "необязательно"}</span>
        </li>
      </ol>
      {preview && (
        <div className="notice notice--info">
          <strong>Будет создано только после подтверждения:</strong>
          <div className="mono">Предметы/00 Входящие</div>
          <div className="field__hint">Папки предметов, недель и ДЗ появятся позднее — при подтверждённой раскладке. Существующие файлы не перезаписываются.</div>
        </div>
      )}
      <p className="field__hint">Демо создаётся в каталоге данных «Контура» и не затрагивает реальные учебные материалы.</p>
    </Dialog>
  );
}
