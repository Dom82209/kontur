import { motion } from "motion/react";
import { CircleCheck, CircleX, Cloud, Copy, FolderOpen, Link2, Lock, Package, Send, ShieldCheck, TriangleAlert } from "lucide-react";
import { useRef, useState } from "react";
import { useOps } from "../../app/operations";
import { useToast } from "../../app/toasts";
import { bridge, type Homework, type PackageItem, type Readiness, type SubmissionPackage, type SubmissionReceipt } from "../../lib/bridge";
import { formatBytes, formatDateTime } from "../../lib/format";
import { Dialog } from "../ui/Dialog";
import { EmptyState } from "../ui/States";

interface Props {
  homework: Homework;
  readiness: Readiness | null;
  pkg: SubmissionPackage | null;
  receipt: SubmissionReceipt | null;
  commentsCount: number;
  onChanged: () => void;
}

const ROLE_TITLE: Record<PackageItem["role"], string> = { main: "Главный итоговый файл", attachment: "Обязательные приложения", extra: "Дополнительные файлы", cloud: "Загрузить на облачный диск" };

function toLocalInput(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** Готовность → комплект → ручное подтверждение отправки → неизменяемый снимок. */
export function SubmissionPanel({ homework: h, readiness, pkg, receipt, commentsCount, onChanged }: Props) {
  const toast = useToast();
  const { reportError, undo } = useOps();
  const [busy, setBusy] = useState(false);
  const [readyOpen, setReadyOpen] = useState(false);
  const [noComments, setNoComments] = useState(false);
  const [submitOpen, setSubmitOpen] = useState(false);
  const [submittedAt, setSubmittedAt] = useState(() => toLocalInput(new Date()));
  const [url, setUrl] = useState(h.assignmentUrl ?? "");
  const [proof, setProof] = useState<{ name: string; sizeBytes: number } | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const proofRef = useRef<HTMLInputElement>(null);
  const isReadyStage = h.workflowStage === "ready_to_submit";

  const act = async <T,>(fn: () => Promise<T>, title: string, ok?: (v: T) => void) => {
    setBusy(true);
    try {
      const v = await fn();
      ok?.(v);
      onChanged();
    } catch (e) {
      reportError(e, title);
    } finally {
      setBusy(false);
    }
  };

  if (receipt) {
    return (
      <section className="panel receipt">
        <div className="panel__head">
          <Lock size={14} className="faint" />
          <h2 className="panel__title">Отправлено · снимок неизменяем</h2>
        </div>
        <div className="panel__body receipt__body">
          <div className="notice notice--ok">
            <CircleCheck />
            <div>
              Отправка подтверждена вами {formatDateTime(receipt.confirmedAt)}. Время отправки: {formatDateTime(receipt.submittedAt)}.
            </div>
          </div>
          <div className="receipt__grid">
            <div>
              <div className="eyebrow">04 Отправлено</div>
              <ul className="pkg__files">
                {receipt.files.map((f) => (
                  <li key={f.id} className="mono">
                    {f.name} <span className="faint">· {formatBytes(f.sizeBytes)}</span>
                  </li>
                ))}
              </ul>
            </div>
            <div>
              <div className="eyebrow">Подтверждение</div>
              <p>{receipt.proof ? <span className="mono">{receipt.proof.name}</span> : <span className="faint">не прикреплено</span>}</p>
              <div className="eyebrow" style={{ marginTop: 8 }}>
                Страница задания
              </div>
              <p className="mono selectable">{receipt.assignmentUrl || "—"}</p>
            </div>
          </div>
          <p className="faint mono" style={{ fontSize: 11 }}>
            {receipt.snapshotPath} · {receipt.snapshotHash}
          </p>
        </div>
      </section>
    );
  }

  const groups = (["main", "attachment", "extra", "cloud"] as const).map((role) => ({ role, items: pkg?.items.filter((i) => i.role === role) ?? [] })).filter((g) => g.items.length > 0);
  const allChecked = pkg?.checklist.every((c) => c.done) ?? false;

  return (
    <div className="submit">
      <section className="panel">
        <div className="panel__head">
          <h2 className="panel__title">Готовность к сдаче</h2>
          {readiness && <span className={`badge ${readiness.ready ? "badge--ok" : "badge--warn"} panel__meta`}>{readiness.ready ? "все проверки пройдены" : "есть невыполненные проверки"}</span>}
        </div>
        <div className="panel__body">
          <ul className="readiness">
            {readiness?.checks.map((c) => (
              <motion.li key={c.id} data-ok={c.ok} layout="position">
                {c.ok ? <CircleCheck size={15} className="readiness__ok" /> : <CircleX size={15} className="readiness__no" />}
                <div>
                  <div className="readiness__label">{c.label}</div>
                  <div className="faint readiness__detail">{c.detail}</div>
                </div>
              </motion.li>
            ))}
          </ul>
          <div className="submit__bar">
            {isReadyStage ? (
              <span className="badge badge--accent">Этап «Готово к сдаче» подтверждён</span>
            ) : (
              <button className="btn btn--primary" disabled={!readiness?.ready || busy} onClick={() => setReadyOpen(true)} title={readiness?.ready ? undefined : "Статус ставится только после всех обязательных проверок"}>
                <ShieldCheck /> Отметить «Готово к сдаче»
              </button>
            )}
            <button className="btn" disabled={!readiness?.ready || busy} onClick={() => void act(() => bridge.buildSubmissionPackage(h.id), "Комплект не собран", (r) => toast.push({ tone: "ok", title: r.operation.title, action: { label: "Отменить", run: () => void undo(r.operation.id) } }))}>
              <Package /> {pkg ? "Пересобрать комплект" : "Собрать комплект"}
            </button>
          </div>
        </div>
      </section>

      <section className="panel">
        <div className="panel__head">
          <h2 className="panel__title">Комплект на сдачу</h2>
          {pkg && <span className="panel__meta faint">собран {formatDateTime(pkg.builtAt)}</span>}
        </div>
        <div className="panel__body">
          {!pkg ? (
            <EmptyState icon={<Package />} title="Комплект ещё не собран" text="После всех проверок «Контур» соберёт итоговый файл, приложения и ссылки в «03 На сдачу» и составит список для загрузки." />
          ) : (
            <div className="pkg">
              {pkg.warnings.length > 0 && (
                <div className="notice notice--warn">
                  <TriangleAlert />
                  <ul>
                    {pkg.warnings.map((w) => (
                      <li key={w}>{w}</li>
                    ))}
                  </ul>
                </div>
              )}
              {groups.map((g) => (
                <div key={g.role} className="pkg__group">
                  <div className="eyebrow">
                    {g.role === "cloud" && <Cloud size={11} />} {ROLE_TITLE[g.role]}
                  </div>
                  <ul className="pkg__files">
                    {g.items.map((i) => (
                      <li key={i.id} data-present={i.present}>
                        <span className="mono">{i.finalName}</span>
                        {i.file && i.file.name !== i.finalName && <span className="faint"> ← {i.file.name}</span>}
                        {!i.present && <span className="badge badge--danger">нет файла</span>}
                      </li>
                    ))}
                  </ul>
                </div>
              ))}
              {pkg.links.length > 0 && (
                <div className="pkg__group">
                  <div className="eyebrow">
                    <Link2 size={11} /> Внешние ссылки
                  </div>
                  <ul className="pkg__files">
                    {pkg.links.map((l) => (
                      <li key={l.id}>
                        {l.label}: <span className="mono selectable">{l.url}</span> {l.verified ? <span className="badge badge--ok">доступна</span> : <span className="badge badge--warn">не проверена</span>}
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              <div className="pkg__group">
                <div className="eyebrow">Прикрепить в систему вуза</div>
                <ol className="pkg__upload">
                  {pkg.uploadList.map((u) => (
                    <li key={u} className="mono">
                      {u}
                    </li>
                  ))}
                </ol>
              </div>

              <ul className="pkg__checklist">
                {pkg.checklist.map((c) => (
                  <li key={c.id}>
                    <label className="confirm" data-auto={c.auto}>
                      <input
                        type="checkbox"
                        className="checkbox"
                        checked={c.done}
                        disabled={c.auto || busy}
                        onChange={(e) => void act(() => bridge.setPackageCheck(pkg.id, c.id, e.target.checked), "Пункт не отмечен")}
                      />
                      <span>
                        {c.label}
                        {c.auto && <span className="faint"> · проверяется автоматически</span>}
                      </span>
                    </label>
                  </li>
                ))}
              </ul>

              <div className="submit__bar">
                <button className="btn" onClick={() => void bridge.revealInFinder(pkg.folderPath).then(() => toast.push({ tone: "info", title: "Папка «03 На сдачу»", text: "Backend откроет её в Finder" }))}>
                  <FolderOpen /> Открыть папку «03 На сдачу»
                </button>
                <button className="btn" onClick={() => void navigator.clipboard?.writeText(pkg.uploadList.join("\n")).then(() => toast.push({ tone: "info", title: "Список файлов скопирован" }))}>
                  <Copy /> Скопировать список файлов
                </button>
                <button className="btn btn--primary" style={{ marginLeft: "auto" }} disabled={!isReadyStage || !allChecked || busy} onClick={() => setSubmitOpen(true)} title={!isReadyStage ? "Сначала отметьте «Готово к сдаче»" : !allChecked ? "Отметьте все пункты чек-листа" : undefined}>
                  <Send /> Отметить как отправленное
                </button>
              </div>
            </div>
          )}
        </div>
      </section>

      <Dialog
        open={readyOpen}
        onClose={() => setReadyOpen(false)}
        title="Отметить «Готово к сдаче»?"
        description="Статус ставится только вами и только после обязательных проверок."
        footer={
          <>
            <button className="btn btn--ghost" onClick={() => setReadyOpen(false)}>
              Отмена
            </button>
            <button
              className="btn btn--primary"
              disabled={busy || (commentsCount === 0 && !noComments)}
              onClick={() =>
                void act(() => bridge.markReadyToSubmit(h.id, { noTeacherComments: noComments }), "Статус не изменён", (r) => {
                  setReadyOpen(false);
                  toast.push({ tone: "ok", title: r.operation.title, action: { label: "Отменить", run: () => void undo(r.operation.id) } });
                })
              }
            >
              <ShieldCheck /> Подтвердить
            </button>
          </>
        }
      >
        <ul className="readiness">
          {readiness?.checks.map((c) => (
            <li key={c.id} data-ok={c.ok}>
              <CircleCheck size={15} className="readiness__ok" />
              <div>
                <div className="readiness__label">{c.label}</div>
                <div className="faint readiness__detail">{c.detail}</div>
              </div>
            </li>
          ))}
        </ul>
        {commentsCount === 0 && (
          <label className="confirm">
            <input type="checkbox" className="checkbox" checked={noComments} onChange={(e) => setNoComments(e.target.checked)} />
            <span>Комментариев преподавателя по этой работе нет.</span>
          </label>
        )}
      </Dialog>

      <Dialog
        open={submitOpen}
        onClose={() => setSubmitOpen(false)}
        title="Подтвердите отправку"
        description="«Контур» ничего не загружает в систему вуза. Это фиксация того, что вы отправили работу сами."
        footer={
          <>
            <button className="btn btn--ghost" onClick={() => setSubmitOpen(false)}>
              Отмена
            </button>
            <button
              className="btn btn--primary"
              disabled={!confirmed || busy}
              onClick={() =>
                void act(() => bridge.confirmSubmission(h.id, { submittedAt: new Date(submittedAt).toISOString(), assignmentUrl: url, proof, userConfirmed: true }), "Отправка не подтверждена", () => {
                  setSubmitOpen(false);
                  toast.push({ tone: "ok", title: "Работа отмечена отправленной", text: "Файлы сохранены в «04 Отправлено» как неизменяемый снимок" });
                })
              }
            >
              <Send /> Подтвердить отправку
            </button>
          </>
        }
      >
        <ul className="pkg__upload">{pkg?.uploadList.map((u) => <li key={u} className="mono">{u}</li>)}</ul>
        <label className="field">
          <span className="field__label">Когда отправлено</span>
          <input type="datetime-local" className="input" value={submittedAt} onChange={(e) => setSubmittedAt(e.target.value)} />
        </label>
        <label className="field">
          <span className="field__label">Ссылка на страницу задания</span>
          <input className="input mono" placeholder="https://…" value={url} onChange={(e) => setUrl(e.target.value)} />
        </label>
        <div className="field">
          <span className="field__label">Скриншот или файл подтверждения</span>
          <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <button className="btn btn--sm" onClick={() => proofRef.current?.click()}>
              Прикрепить
            </button>
            <span className="faint mono">{proof?.name ?? "необязательно"}</span>
          </div>
          <input
            ref={proofRef}
            type="file"
            hidden
            accept="image/*,.pdf"
            onChange={(e) => {
              const f = e.target.files?.[0];
              e.target.value = "";
              if (f) setProof({ name: f.name, sizeBytes: f.size });
            }}
          />
        </div>
        <label className="confirm">
          <input type="checkbox" className="checkbox" checked={confirmed} onChange={(e) => setConfirmed(e.target.checked)} />
          <span>Я сама загрузила эти файлы в систему вуза. После подтверждения отправленная версия станет неизменяемым снимком — отменить нельзя.</span>
        </label>
      </Dialog>
    </div>
  );
}
