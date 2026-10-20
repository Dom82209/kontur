import { useCallback, useEffect, useState } from "react";
import { useOps } from "../../app/operations";
import { useToast } from "../../app/toasts";
import { bridge, type AutomationRules } from "../bridge";
import { useBridgeQuery } from "./useBridgeQuery";

/** Черновик правил автоматизации: правки копятся локально и сохраняются одной операцией. */
export function useRulesDraft() {
  const q = useBridgeQuery(() => bridge.getAutomationRules(), [], ["operation:completed"]);
  const { reportError, undo } = useOps();
  const toast = useToast();
  const [draft, setDraft] = useState<AutomationRules | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (q.data && !draft) setDraft(q.data);
  }, [q.data, draft]);

  const dirty = !!draft && !!q.data && JSON.stringify(draft) !== JSON.stringify(q.data);

  const update = useCallback((patch: Partial<AutomationRules> | ((r: AutomationRules) => AutomationRules)) => {
    setDraft((d) => (d ? (typeof patch === "function" ? patch(d) : { ...d, ...patch }) : d));
  }, []);

  const save = async () => {
    if (!draft) return;
    setSaving(true);
    try {
      const res = await bridge.updateAutomationRules(draft);
      setDraft(res.value);
      q.setData(() => res.value);
      toast.push({ tone: "ok", title: "Правила сохранены", text: "Перед изменением создана резервная копия", action: { label: "Отменить", run: () => void undo(res.operation.id).then(() => setDraft(null)) } });
    } catch (e) {
      reportError(e, "Правила не сохранены");
    } finally {
      setSaving(false);
    }
  };

  return { rules: draft, saved: q.data, loading: q.loading, error: q.error, reload: q.reload, dirty, saving, update, save, reset: () => setDraft(q.data) };
}
