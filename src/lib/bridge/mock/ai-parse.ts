import type { AiDraft, AiIssue, AiParseStatus, AiTask } from "../ai-types";

/**
 * Зеркало разбора ответа backend (`ai/response.rs`) для демонстрационного режима:
 * JSON целиком, в блоке ```json или первый сбалансированный объект; частичный ответ
 * даёт черновик с пропусками и замечаниями, повреждённый — пустой черновик.
 */
export function findJsonObject(text: string): Record<string, unknown> | null {
  const tryParse = (raw: string): Record<string, unknown> | null => {
    try {
      const value: unknown = JSON.parse(raw);
      return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
    } catch {
      return null;
    }
  };
  const trimmed = text.trim();
  const whole = tryParse(trimmed);
  if (whole) return whole;
  const fence = /```[a-zA-Z]*\n([\s\S]*?)```/g;
  for (let m = fence.exec(trimmed); m; m = fence.exec(trimmed)) {
    const inner = tryParse(m[1]!.trim());
    if (inner) return inner;
  }
  for (let start = trimmed.indexOf("{"), attempts = 0; start >= 0 && attempts < 20; start = trimmed.indexOf("{", start + 1), attempts += 1) {
    let depth = 0;
    let inString = false;
    let escaped = false;
    for (let i = start; i < trimmed.length; i += 1) {
      const c = trimmed[i];
      if (inString) {
        if (escaped) escaped = false;
        else if (c === "\\") escaped = true;
        else if (c === '"') inString = false;
        continue;
      }
      if (c === '"') inString = true;
      else if (c === "{") depth += 1;
      else if (c === "}") {
        depth -= 1;
        if (depth === 0) {
          const candidate = tryParse(trimmed.slice(start, i + 1));
          if (candidate) return candidate;
          break;
        }
      }
    }
  }
  return null;
}

const VERDICTS = ["pass", "needs_fixes", "needs_teacher_review", "blocked"];
const STATUSES = ["pass", "warn", "fail", "manual", "na"];

export function parseAiResponse(task: AiTask, raw: string, expectsPrompt: boolean): { status: AiParseStatus; draft: AiDraft; issues: AiIssue[] } {
  if (!raw.trim()) return { status: "empty", draft: {}, issues: [{ code: "empty_response", message: "Ответ пустой" }] };
  const object = findJsonObject(raw);
  if (!object) {
    if (task === "initial_prompt") return { status: "partial", draft: { promptText: raw.trim() }, issues: [{ code: "plain_text", message: "Ответ без JSON — весь текст взят как промт; проверьте его" }] };
    if (task === "revision_prompt") return { status: "partial", draft: { text: raw.trim() }, issues: [{ code: "plain_text", message: "Ответ без JSON — весь текст взят как промт; проверьте его" }] };
    return { status: "corrupt", draft: {}, issues: [{ code: "no_json", message: "В ответе нет JSON-объекта нужного формата — сырой текст сохранён, заполните результат вручную" }] };
  }
  const draft: Record<string, unknown> = {};
  const issues: AiIssue[] = [];
  const missing: string[] = [];
  const used = new Set<string>();
  const take = (keys: string[]) => {
    for (const k of keys) if (k in object) {
      used.add(k);
      return object[k];
    }
    return undefined;
  };
  const text = (field: string, aliases: string[], required: boolean) => {
    const v = take([field, ...aliases]);
    if (typeof v === "string" && v.trim()) draft[field] = v.trim();
    else {
      if (v !== undefined && typeof v !== "string") issues.push({ code: "invalid_field", message: `Поле «${field}» должно быть строкой` });
      if (required) missing.push(field);
    }
  };
  const choice = (field: string, aliases: string[], allowed: string[]) => {
    const v = take([field, ...aliases]);
    const n = typeof v === "string" ? v.trim().toLowerCase().replace(/[- ]/g, "_") : null;
    if (n && allowed.includes(n)) draft[field] = n;
    else {
      if (v !== undefined) issues.push({ code: "invalid_value", message: `«${field}»: недопустимое значение` });
      missing.push(field);
    }
  };
  if (task === "material_analysis") {
    text("summary", ["analysis"], true);
    text("promptText", ["prompt_text", "prompt"], expectsPrompt);
  } else if (task === "initial_prompt") text("promptText", ["prompt_text", "prompt", "text"], true);
  else if (task === "revision_prompt") text("text", ["promptText", "prompt_text", "prompt"], true);
  else {
    choice("verdict", [], VERDICTS);
    text("summary", [], true);
    choice("correctness", [], STATUSES);
    choice("taskFit", ["task_fit"], STATUSES);
    choice("courseGrounding", ["course_grounding"], STATUSES);
    choice("teacher", [], STATUSES);
    const score = take(["styleScore", "style_score"]);
    if (typeof score === "number" && score >= 0 && score <= 10) draft.styleScore = Math.round(score);
    else {
      if (score !== undefined && score !== null) issues.push({ code: "invalid_value", message: "Расхождение со стилем должно быть числом 0–10 — поле оставлено пустым" });
      draft.styleScore = null;
    }
    const list = (field: string, criterion: string) => {
      const v = take([field]);
      draft[field] = Array.isArray(v)
        ? v
            .map((item) => (typeof item === "string" ? { text: item } : (item as Record<string, unknown>)))
            .filter((item) => typeof item?.text === "string" && (item.text as string).trim())
            .map((item) => ({ criterion: typeof item.criterion === "string" ? item.criterion : criterion, text: (item.text as string).trim(), location: typeof item.location === "string" ? item.location : null }))
        : [];
    };
    list("critical", "correctness");
    list("remarks", "task_fit");
    const claims = take(["unsupportedClaims", "unsupported_claims"]);
    draft.unsupportedClaims = Array.isArray(claims)
      ? claims.map((c) => (typeof c === "string" ? { text: c, location: "место не указано" } : { text: String((c as Record<string, unknown>).text ?? ""), location: String((c as Record<string, unknown>).location ?? "место не указано") })).filter((c) => c.text.trim())
      : [];
    const miss = take(["missing"]);
    draft.missing = Array.isArray(miss)
      ? miss.map((m) => (typeof m === "string" ? { kind: /http|ссылк/i.test(m) ? "link" : "file", label: m, required: true } : { kind: (m as Record<string, unknown>).kind === "link" ? "link" : "file", label: String((m as Record<string, unknown>).label ?? ""), required: (m as Record<string, unknown>).required !== false })).filter((m) => m.label.trim())
      : [];
  }
  const unknown = Object.keys(object).filter((k) => !used.has(k));
  if (unknown.length) issues.push({ code: "unknown_fields", message: `Лишние поля проигнорированы: ${unknown.join(", ")}` });
  for (const f of missing) issues.push({ code: "missing_field", message: `Нет обязательного поля «${f}» — заполните его вручную` });
  const recognized = Object.values(draft).some((v) => (typeof v === "string" ? v.length > 0 : Array.isArray(v) ? v.length > 0 : v !== null && v !== undefined));
  const status: AiParseStatus = !recognized ? "corrupt" : missing.length === 0 && issues.length === 0 ? "valid" : "partial";
  return { status, draft: draft as AiDraft, issues };
}
