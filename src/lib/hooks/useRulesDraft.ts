import { useCallback, useEffect, useRef, useState } from "react";
import { useOps } from "../../app/operations";
import { useToast } from "../../app/toasts";
import { bridge, type AutomationRules } from "../bridge";
import { useBridgeQuery } from "./useBridgeQuery";

const same = (a: AutomationRules | null, b: AutomationRules | null) => JSON.stringify(a) === JSON.stringify(b);

/**
 * Черновик правил автоматизации: правки копятся локально и сохраняются одной
 * операцией. Без несохранённых правок черновик всегда следует за сохранёнными
 * правилами backend (например, после изменения ключевых слов в «Предметах»);
 * с правками — не затирается, пока пользователь не сохранит или не сбросит их.
 */
export function useRulesDraft() {
  const q = useBridgeQuery(() => bridge.getAutomationRules(), [], ["operation:completed"]);
  const { reportError } = useOps();
  const toast = useToast();
  const [draft, setDraft] = useState<AutomationRules | null>(null);
  const [saving, setSaving] = useState(false);
  const base = useRef<AutomationRules | null>(null);

  useEffect(() => {
    if (!q.data) return;
    setDraft((current) => {
      const pristine = current === null || same(current, base.current);
      base.current = q.data;
      return pristine ? q.data : current;
    });
  }, [q.data]);

  const dirty = !!draft && !!q.data && !same(draft, q.data);

  const update = useCallback((patch: Partial<AutomationRules> | ((r: AutomationRules) => AutomationRules)) => {
    setDraft((d) => (d ? (typeof patch === "function" ? patch(d) : { ...d, ...patch }) : d));
  }, []);

  const save = async () => {
    if (!draft) return;
    setSaving(true);
    try {
      const res = await bridge.updateAutomationRules(draft);
      base.current = res.value;
      setDraft(res.value);
      q.setData(() => res.value);
      toast.push({ tone: "ok", title: "Правила сохранены", text: "Действуют сразу и сохраняются после перезапуска" });
    } catch (e) {
      reportError(e, "Правила не сохранены");
    } finally {
      setSaving(false);
    }
  };

  return { rules: draft, saved: q.data, loading: q.loading, error: q.error, reload: q.reload, dirty, saving, update, save, reset: () => setDraft(q.data) };
}
