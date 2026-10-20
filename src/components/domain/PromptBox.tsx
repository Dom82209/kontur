import { Copy, Send } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { useToast } from "../../app/toasts";
import type { RevisionPrompt } from "../../lib/bridge";
import { formatDateTime, providerName } from "../../lib/format";

const KIND: Record<RevisionPrompt["kind"], string> = { initial: "Первичный промт", revision: "Промт исправлений", teacher: "Промт по комментариям преподавателя" };

interface Props {
  prompt: RevisionPrompt;
  editable?: boolean;
  onChange?: (text: string) => void;
  actions?: ReactNode;
  compact?: boolean;
}

/** Текст промта с источниками. Отправленный промт только для чтения. */
export function PromptBox({ prompt, editable, onChange, actions, compact }: Props) {
  const toast = useToast();
  const [text, setText] = useState(prompt.text);
  useEffect(() => setText(prompt.text), [prompt.id, prompt.text]);
  const canEdit = editable && !prompt.sentAt;
  return (
    <div className="promptbox" data-compact={compact ?? false}>
      <div className="promptbox__head">
        <span className="promptbox__kind">{KIND[prompt.kind]}</span>
        <span className="faint">
          {prompt.createdBy === "user" ? "вы" : providerName(prompt.createdBy)} · {formatDateTime(prompt.createdAt)}
        </span>
        {prompt.sentAt && (
          <span className="badge badge--ok">
            <Send /> передан
          </span>
        )}
        <button
          className="btn btn--sm btn--ghost"
          style={{ marginLeft: "auto" }}
          onClick={() => void navigator.clipboard?.writeText(text).then(() => toast.push({ tone: "info", title: "Промт скопирован" }))}
        >
          <Copy /> Скопировать промт
        </button>
      </div>
      {canEdit ? (
        <textarea
          className="textarea promptbox__text"
          value={text}
          aria-label="Текст промта"
          spellCheck={false}
          onChange={(e) => {
            setText(e.target.value);
            onChange?.(e.target.value);
          }}
        />
      ) : (
        <pre className="promptbox__text promptbox__text--ro selectable">{text}</pre>
      )}
      {prompt.sources.length > 0 && (
        <div className="promptbox__sources faint">
          Источники ({prompt.sources.length}): {prompt.sources.map((s) => s.fileName).join(" · ")}
        </div>
      )}
      {actions && <div className="promptbox__actions">{actions}</div>}
    </div>
  );
}
