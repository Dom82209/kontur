import { ArrowRight, RotateCw, ShieldCheck, TriangleAlert } from "lucide-react";
import { useCallback, useState, type ReactNode } from "react";
import { useOps } from "../../app/operations";
import { bridge, type Workspace, type WorkspaceKind, type WorkspaceTransitionPreview, type WorkspaceTransitionRequest } from "../../lib/bridge";
import { Dialog } from "../ui/Dialog";

const ACTION_TITLE: Record<WorkspaceTransitionRequest["kind"], string> = {
  create_real: "Создать рабочее пространство",
  connect_existing: "Подключить существующее пространство",
  create_demo: "Создать демо-пространство",
  activate: "Переключить рабочее пространство",
};

export function workspaceKindLabel(kind: WorkspaceKind | undefined | null): string {
  if (kind === "real") return "реальное";
  if (kind === "demo") return "демо";
  return "не создано";
}

export function workspaceLabel(workspace: Workspace | null | undefined): string {
  if (!workspace) return "нет";
  const vault = workspace.vaultCanonical?.split("/").filter(Boolean).pop();
  return `${workspaceKindLabel(workspace.kind)}${vault ? ` · ${vault}` : ""}`;
}

type Phase = { kind: "idle" } | { kind: "preview"; preview: WorkspaceTransitionPreview } | { kind: "restarting"; workspace: Workspace };

/**
 * Переход между пространствами всегда проходит preview → подтверждение/отмена.
 * Backend повторно проверяет vault при подтверждении и перезапускает приложение.
 */
export function useWorkspaceTransition(onApplied?: () => void): { start: (request: WorkspaceTransitionRequest) => Promise<void>; busy: boolean; dialog: ReactNode } {
  const { reportError } = useOps();
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [busy, setBusy] = useState(false);

  const start = useCallback(
    async (request: WorkspaceTransitionRequest) => {
      setBusy(true);
      try {
        const preview = await bridge.previewWorkspaceTransition(request);
        if (preview) setPhase({ kind: "preview", preview });
      } catch (error) {
        reportError(error, `${ACTION_TITLE[request.kind]}: предпросмотр не выполнен`);
      } finally {
        setBusy(false);
      }
    },
    [reportError],
  );

  const cancel = async () => {
    if (phase.kind !== "preview") return setPhase({ kind: "idle" });
    const id = phase.preview.id;
    setPhase({ kind: "idle" });
    try {
      await bridge.cancelWorkspaceTransition(id);
    } catch {
      // Уже применён/устарел — отмена не требуется.
    }
  };

  const confirm = async () => {
    if (phase.kind !== "preview") return;
    setBusy(true);
    try {
      const activation = await bridge.confirmWorkspaceTransition(phase.preview.id);
      setPhase({ kind: "restarting", workspace: activation.workspace });
      onApplied?.();
    } catch (error) {
      setPhase({ kind: "idle" });
      reportError(error, `${ACTION_TITLE[phase.preview.action]}: не выполнено`);
    } finally {
      setBusy(false);
    }
  };

  const dialog =
    phase.kind === "preview" ? (
      <Dialog
        open
        onClose={() => void cancel()}
        title={ACTION_TITLE[phase.preview.action]}
        description="Ничего не изменено. Подтвердите, чтобы применить; «Контур» повторно проверит vault и перезапустится."
        wide
        footer={
          <>
            <button className="btn btn--ghost" disabled={busy} onClick={() => void cancel()} data-close>
              Отмена
            </button>
            <button className="btn btn--primary" disabled={busy} aria-busy={busy} onClick={() => void confirm()}>
              <ShieldCheck /> Подтвердить и перезапустить
            </button>
          </>
        }
      >
        <TransitionPreviewBody preview={phase.preview} />
      </Dialog>
    ) : phase.kind === "restarting" ? (
      <Dialog open onClose={() => setPhase({ kind: "idle" })} title="Пространство переключено">
        <p>
          <RotateCw /> Активно: <strong>{workspaceLabel(phase.workspace)}</strong>. Приложение перезапускается, чтобы открыть базу этого пространства.
        </p>
      </Dialog>
    ) : null;

  return { start, busy, dialog };
}

export function TransitionPreviewBody({ preview }: { preview: WorkspaceTransitionPreview }) {
  return (
    <div className="workspace-preview" data-testid="workspace-preview">
      <div className="workspace-preview__route">
        <span>{workspaceLabel(preview.currentWorkspace)}</span>
        <ArrowRight aria-label="станет" />
        <strong>{preview.targetWorkspace ? workspaceLabel(preview.targetWorkspace) : `новое ${workspaceKindLabel(preview.targetKind)}`}</strong>
      </div>
      {preview.vaultDisplayPath && (
        <div className="field__hint">
          Vault: <span className="mono selectable">{preview.vaultDisplayPath}</span>
        </div>
      )}
      <strong>После подтверждения:</strong>
      <ul>
        {preview.effects.map((effect) => (
          <li key={effect}>{effect}</li>
        ))}
      </ul>
      {preview.warnings.map((warning) => (
        <div key={warning} className="notice notice--warn">
          <TriangleAlert /> {warning}
        </div>
      ))}
      <p className="field__hint">Предпросмотр действует до {new Date(preview.expiresAt).toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" })}.</p>
    </div>
  );
}
