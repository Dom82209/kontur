import { describe, expect, it } from "vitest";
import { providerAccountMode } from "./format";

/** Этап 12: подписи режима аккаунта не выдают ошибки CLI за «нет авторизации». */
describe("режим аккаунта провайдера", () => {
  it("различает подписку, ручную передачу, вход, отсутствие CLI и ошибку", () => {
    expect(providerAccountMode({ availability: "connected", state: "connected" })).toEqual({ label: "подписка аккаунта", tone: "ok" });
    expect(providerAccountMode({ availability: "manual", state: "disconnected" }).label).toBe("ручная передача");
    expect(providerAccountMode({ availability: "error", state: "reauth_required" }).label).toBe("не авторизован");
    expect(providerAccountMode({ availability: "unavailable", state: "error" }).label).toBe("CLI недоступен — ручная передача");
    expect(providerAccountMode({ availability: "error", state: "error" }).label).toBe("ошибка проверки CLI");
  });
});
