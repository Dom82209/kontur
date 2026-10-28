import { describe, expect, it } from "vitest";
import { MockDesktopBridge } from "./MockDesktopBridge";

describe("Mock: рабочие пространства (preview → confirm/cancel)", () => {
  it("preview ничего не меняет, confirm одноразовый", async () => {
    const bridge = new MockDesktopBridge();
    const before = await bridge.getManagedRoots();
    const preview = await bridge.previewWorkspaceTransition({ kind: "create_real" });
    expect(preview.effects.length).toBeGreaterThan(0);
    expect((await bridge.getManagedRoots()).workspace).toEqual(before.workspace);

    const activation = await bridge.confirmWorkspaceTransition(preview.id);
    expect(activation.restartRequired).toBe(true);
    expect(activation.workspace.kind).toBe("real");
    expect((await bridge.getManagedRoots()).workspace?.id).toBe(activation.workspace.id);
    await expect(bridge.confirmWorkspaceTransition(preview.id)).rejects.toMatchObject({ code: "conflict" });
  });

  it("отменённый preview нельзя подтвердить", async () => {
    const bridge = new MockDesktopBridge();
    const preview = await bridge.previewWorkspaceTransition({ kind: "create_demo" });
    await bridge.cancelWorkspaceTransition(preview.id);
    await expect(bridge.confirmWorkspaceTransition(preview.id)).rejects.toMatchObject({ code: "conflict" });
    const overview = await bridge.getWorkspaces();
    expect(overview.recentTransitions[0]).toMatchObject({ id: preview.id, state: "cancelled" });
  });

  it("переключение назад на демо и запрет активировать текущее", async () => {
    const bridge = new MockDesktopBridge();
    const demo = (await bridge.getWorkspaces()).active!;
    await expect(bridge.previewWorkspaceTransition({ kind: "activate", workspaceId: demo.id })).rejects.toMatchObject({ code: "conflict" });
    const real = await bridge.previewWorkspaceTransition({ kind: "create_real" });
    await bridge.confirmWorkspaceTransition(real.id);
    const back = await bridge.previewWorkspaceTransition({ kind: "activate", workspaceId: demo.id });
    expect(back.targetWorkspace?.kind).toBe("demo");
    await bridge.confirmWorkspaceTransition(back.id);
    expect((await bridge.getWorkspaces()).active?.id).toBe(demo.id);
  });
});
