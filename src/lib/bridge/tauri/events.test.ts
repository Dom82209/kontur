import { afterEach, describe, expect, it, vi } from "vitest";

import {
  __setListenForTests,
  __setRegistrationErrorHandlerForTests,
  listenToBridgeEvent,
} from "./events";
import { __setTauriRuntimeForTests } from "./runtime";

afterEach(() => {
  __setListenForTests(undefined);
  __setRegistrationErrorHandlerForTests(undefined);
  __setTauriRuntimeForTests(undefined);
});

describe("Tauri event adapter", () => {
  it("передаёт listener типизированный payload", async () => {
    let emit: ((message: { payload: unknown }) => void) | undefined;
    __setListenForTests(async (_event, handler) => {
      emit = handler;
      return () => undefined;
    });
    const listener = vi.fn();
    listenToBridgeEvent("operation:progress", listener);
    await vi.waitFor(() => expect(emit).toBeTypeOf("function"));

    emit?.({ payload: { operationId: "op-1", kind: "route", progress: 0.5, message: "Готово" } });
    expect(listener).toHaveBeenCalledWith({
      operationId: "op-1",
      kind: "route",
      progress: 0.5,
      message: "Готово",
    });
  });

  it("обычная отписка вызывает настоящий Tauri unlisten и блокирует callback", async () => {
    let emit: ((message: { payload: unknown }) => void) | undefined;
    const nativeUnlisten = vi.fn();
    __setListenForTests(async (_event, handler) => {
      emit = handler;
      return nativeUnlisten;
    });
    const listener = vi.fn();
    const unsubscribe = listenToBridgeEvent("inbox:file-added", listener);
    await vi.waitFor(() => expect(emit).toBeTypeOf("function"));

    unsubscribe();
    emit?.({ payload: { files: [] } });
    expect(nativeUnlisten).toHaveBeenCalledOnce();
    expect(listener).not.toHaveBeenCalled();
  });

  it("ранняя отписка удаляет подписку сразу после асинхронной регистрации", async () => {
    let resolveRegistration: ((unlisten: () => void) => void) | undefined;
    const nativeUnlisten = vi.fn();
    __setListenForTests(
      () => new Promise((resolve) => {
        resolveRegistration = resolve;
      }),
    );
    const unsubscribe = listenToBridgeEvent("review:completed", vi.fn());
    unsubscribe();
    await vi.waitFor(() => expect(resolveRegistration).toBeTypeOf("function"));

    resolveRegistration?.(nativeUnlisten);
    await vi.waitFor(() => expect(nativeUnlisten).toHaveBeenCalledOnce());
  });

  it("повторный unsubscribe безопасен", async () => {
    const nativeUnlisten = vi.fn();
    let registered = false;
    __setListenForTests(async () => {
      registered = true;
      return nativeUnlisten;
    });
    const unsubscribe = listenToBridgeEvent("submission:confirmed", vi.fn());
    await vi.waitFor(() => expect(registered).toBe(true));
    await Promise.resolve();

    unsubscribe();
    unsubscribe();
    expect(nativeUnlisten).toHaveBeenCalledOnce();
  });

  it("ошибка регистрации контролируемо передаётся error handler без unhandled rejection", async () => {
    const onError = vi.fn();
    __setRegistrationErrorHandlerForTests(onError);
    __setListenForTests(async () => Promise.reject({ code: "database", message: "listener failed" }));

    expect(() => listenToBridgeEvent("operation:failed", vi.fn())).not.toThrow();
    await vi.waitFor(() => expect(onError).toHaveBeenCalledOnce());
    expect(onError.mock.calls[0]?.[0]).toMatchObject({ code: "database", message: "listener failed" });
  });

  it("вне Tauri runtime возвращает контролируемый backend_unavailable", async () => {
    const onError = vi.fn();
    __setRegistrationErrorHandlerForTests(onError);
    __setListenForTests(null);
    __setTauriRuntimeForTests(false);

    const unsubscribe = listenToBridgeEvent("provider:status-changed", vi.fn());
    await vi.waitFor(() => expect(onError).toHaveBeenCalledOnce());
    expect(onError.mock.calls[0]?.[0]).toMatchObject({ code: "backend_unavailable" });
    expect(() => unsubscribe()).not.toThrow();
  });

  it("browser runtime не требует существования Tauri globals", async () => {
    const onError = vi.fn();
    __setRegistrationErrorHandlerForTests(onError);
    __setTauriRuntimeForTests(false);

    listenToBridgeEvent("workflow:stage-changed", vi.fn());
    await vi.waitFor(() => expect(onError).toHaveBeenCalledOnce());
    expect(onError.mock.calls[0]?.[0]).toMatchObject({ code: "backend_unavailable" });
  });
});
