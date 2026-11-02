import { describe, expect, it } from "vitest";
import { MockDesktopBridge } from "./MockDesktopBridge";
import type { CommentAuthority } from "../types";

/** Этап 7: mock повторяет правила backend об авторитетности и устаревании. */
const HW = "hw-linal-8";
const answer = (authority: CommentAuthority, confirmedOfficial: boolean) => ({
  source: "chat" as const,
  date: "2026-10-22T12:00:00Z",
  question: "Нужно ли приложить расчёт?",
  answer: "Да, отдельным листом",
  authority,
  author: "Автор",
  context: "https://t.me/c/1/2",
  confirmedOfficial,
});

describe("авторитетность ответов (этап 7, mock)", () => {
  it("официальным может быть только подтверждённый ответ преподавателя или ассистента", async () => {
    const b = new MockDesktopBridge({ latency: 0 });
    for (const authority of ["student", "group_chat_unverified", "user_note", "system", "unknown"] as const) {
      await expect(b.saveTeacherComments(HW, [answer(authority, true)])).rejects.toMatchObject({ code: "blocked" });
      const [saved] = (await b.saveTeacherComments(HW, [answer(authority, false)])).value;
      expect(saved!.official).toBe(false);
      await expect(b.addTeacherRequirement(HW, { commentId: saved!.id, text: "Расчёт", origin: "Да", status: "done" })).rejects.toMatchObject({ code: "blocked" });
    }
    for (const authority of ["teacher_official", "teaching_assistant"] as const) {
      const [saved] = (await b.saveTeacherComments(HW, [answer(authority, true)])).value;
      expect(saved).toMatchObject({ official: true, author: "Автор", context: "https://t.me/c/1/2" });
    }
  });

  it("отзыв официальности отзывает требования и меняет отпечаток; история сохраняется", async () => {
    const b = new MockDesktopBridge({ latency: 0 });
    const [saved] = (await b.saveTeacherComments(HW, [answer("teacher_official", true)])).value;
    const before = (await b.getWorkflow(HW))!.inputs.combined;
    await b.setCommentAuthority(saved!.id, { authority: "group_chat_unverified", author: "", context: "", confirmedOfficial: false });
    const wf = (await b.getWorkflow(HW))!;
    expect(wf.inputs.combined).not.toBe(before);
    expect(wf.teacherComments.find((c) => c.id === saved!.id)?.official).toBe(false);
    await expect(b.setCommentAuthority(saved!.id, { authority: "student", author: "", context: "", confirmedOfficial: true })).rejects.toMatchObject({ code: "blocked" });
  });

  it("в демо «Готово к сдаче» не устарело, а новый официальный ответ снимает отметку", async () => {
    const b = new MockDesktopBridge({ latency: 0 });
    const ready = (await b.getWorkflow("hw-stat-7"))!;
    expect(ready.readyStale).toBe(false);
    if (ready.stage === "ready_to_submit") {
      await b.saveTeacherComments("hw-stat-7", [answer("teacher_official", true)]);
      expect((await b.getWorkflow("hw-stat-7"))!.stage).not.toBe("ready_to_submit");
    }
  });
});
