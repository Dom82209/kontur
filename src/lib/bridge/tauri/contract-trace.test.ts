import { describe, expect, it } from "vitest";
import ipc from "./ipc.ts?raw";
import rust from "../../../../src-tauri/src/ipc_commands.rs?raw";

/**
 * Этап 13: трассировка контракта. Каждая Tauri-команда backend имеет обёртку
 * в ipc.ts, и каждая обёртка вызывает существующую команду. Регистрация в
 * handler и capability проверяется тестами Rust.
 */
describe("трассировка IPC-контракта", () => {
  it("команды ipc.ts и src-tauri/src/ipc_commands.rs совпадают", () => {
    const frontend = new Set([...ipc.matchAll(/call(?:<[^>]*>)?\("([a-z_]+)"/g)].map((m) => m[1]));
    const backend = new Set([...rust.slice(rust.indexOf("COMMANDS")).matchAll(/"([a-z_]+)"/g)].map((m) => m[1]));
    expect([...frontend].filter((c) => !backend.has(c))).toEqual([]);
    expect([...backend].filter((c) => !frontend.has(c))).toEqual([]);
    expect(backend.size).toBeGreaterThan(100);
  });
});
