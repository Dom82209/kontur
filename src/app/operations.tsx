import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";
import { bridge, BridgeError, type Operation, type OperationKind } from "../lib/bridge";
import { useBridgeEvent } from "../lib/hooks/useBridgeEvent";
import { useToast } from "./toasts";

interface RunningOp {
  id: string;
  kind: OperationKind;
  progress: number;
  message: string;
}

interface OpsApi {
  running: RunningOp | null;
  lastUndoable: Operation | null;
  undo: (operationId?: string) => Promise<void>;
  /** Унифицированная обработка ошибок bridge в тосты. */
  reportError: (e: unknown, title?: string) => void;
}

const Ctx = createContext<OpsApi | null>(null);

const FILE_OPS: OperationKind[] = ["route", "create_week", "create_version", "mark_final", "mark_submitted", "update_rules", "save_teacher_comments", "build_package", "mark_ready"];

export function OperationsProvider({ children }: { children: ReactNode }) {
  const toast = useToast();
  const [running, setRunning] = useState<RunningOp | null>(null);
  const [lastUndoable, setLastUndoable] = useState<Operation | null>(null);

  useBridgeEvent("operation:progress", (p) => {
    setRunning({ id: p.operationId, kind: p.kind, progress: p.progress, message: p.message });
  });
  useBridgeEvent("operation:completed", ({ operation }) => {
    setRunning((r) => (r?.id === operation.id ? null : r));
    if (operation.undoable && FILE_OPS.includes(operation.kind)) setLastUndoable(operation);
  });
  useBridgeEvent("operation:failed", ({ operation, error }) => {
    setRunning((r) => (r?.id === operation.id ? null : r));
    toast.push({ tone: "danger", title: operation.title, text: error.message });
  });

  const reportError = useCallback(
    (e: unknown, title = "Операция не выполнена") => {
      const text = e instanceof BridgeError || e instanceof Error ? e.message : "Неизвестная ошибка";
      toast.push({ tone: e instanceof BridgeError && e.code === "confirmation_required" ? "warn" : "danger", title, text });
    },
    [toast],
  );

  const undo = useCallback(
    async (operationId?: string) => {
      let id = operationId;
      if (!id) {
        const log = await bridge.getActivityLog();
        id = log.find((o) => o.undoable && o.status === "completed" && FILE_OPS.includes(o.kind))?.id;
      }
      if (!id) {
        toast.push({ tone: "info", title: "Нечего отменять", text: "Последние операции уже отменены или необратимы" });
        return;
      }
      try {
        const res = await bridge.undoOperation(id);
        setLastUndoable((op) => (op?.id === id ? null : op));
        toast.push({ tone: "ok", title: res.operation.title, text: "Файлы восстановлены из резервной копии" });
      } catch (e) {
        reportError(e, "Отмена не выполнена");
      }
    },
    [toast, reportError],
  );

  const value = useMemo(() => ({ running, lastUndoable, undo, reportError }), [running, lastUndoable, undo, reportError]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useOps(): OpsApi {
  const v = useContext(Ctx);
  if (!v) throw new Error("useOps вне OperationsProvider");
  return v;
}
