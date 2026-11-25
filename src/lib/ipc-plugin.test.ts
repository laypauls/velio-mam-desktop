// Categories: Happy (T-ui-ipc-plugin-01, 04) / Boundary (02 fixture shapes) / Validation (03 error codes, 05 enum lists) / External (01 rejection, 03)
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { beforeEach, describe, expect, it, vi } from "vitest"
import { createMockApi } from "@/lib/api/mock"
import { errorKey, isCommandError } from "@/lib/errors"
import type { CommandError, JobKind } from "@/lib/ipc-contract"
import {
  PLUGIN_CHANGED_EVENT,
  PLUGIN_PERMISSIONS,
  onPluginChanged,
  pluginContributions,
  pluginExport,
  pluginGrantPermissions,
  pluginInspectDir,
  pluginInstall,
  pluginList,
  pluginRetryFailed,
  pluginRunAll,
  pluginRunCommand,
  pluginSetEnabled,
  pluginTaskCounts,
  pluginUninstall,
  type ChangeReason,
  type PluginChanged,
  type PluginContributions,
  type PluginErrorCode,
  type PluginInfo,
  type PluginPermission,
  type PluginPreview,
  type PluginTaskCount,
} from "./ipc"
import fixtureJson from "../../src-tauri/src/plugin_ipc_fixture.json"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

/** 공유 픽스처 — Rust 쪽 테스트가 같은 파일을 검증한다. JSON 리터럴 타입(string)을 계약 타입으로 좁혀 tsc 가 모양을 검사한다 */
const fixture = fixtureJson as {
  permissions: PluginPermission[]
  errorCodes: PluginErrorCode[]
  pluginInfo: PluginInfo
  pluginInfoBroken: PluginInfo
  pluginPreview: PluginPreview
  pluginContributions: PluginContributions
  commandResult: { message: string | null }
  commandResultNull: { message: string | null }
  runAllResult: { started: boolean }
  exportResult: { bytesWritten: number }
  taskCounts: PluginTaskCount[]
  changedEvent: PluginChanged
  changeReasons: ChangeReason[]
  eventName: string
}

/** 컴파일 타임 완전성: 새 필드가 계약에 생기면 이 리터럴이 깨진다 */
const INFO_KEYS: Record<keyof PluginInfo, true> = {
  id: true, version: true, displayName: true, description: true, status: true, enabled: true, unsigned: true,
  permissions: true, granted: true, commands: true, providers: true, installedAt: true, workerAvailable: true,
}
const PREVIEW_KEYS: Record<keyof PluginPreview, true> = {
  id: true, version: true, displayName: true, description: true, permissions: true, commands: true, providers: true,
  unsigned: true, installed: true, change: true, addedPermissions: true, hasKeptData: true,
}

const keysOf = (o: object) => Object.keys(o).sort()

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(listen).mockReset()
})

describe("plugin ipc wrappers", () => {
  const info = fixture.pluginInfo
  const cases: { name: string; call: () => Promise<unknown>; command: string; args?: Record<string, unknown> }[] = [
    { name: "pluginList", call: () => pluginList(), command: "plugin_list" },
    { name: "pluginInspectDir", call: () => pluginInspectDir("C:\\p"), command: "plugin_inspect_dir", args: { dir: "C:\\p" } },
    {
      name: "pluginInstall",
      call: () => pluginInstall("C:\\p", "a.b", "1.0.0", ["kv.read"]),
      command: "plugin_install",
      args: { dir: "C:\\p", expectedId: "a.b", expectedVersion: "1.0.0", grantedPermissions: ["kv.read"] },
    },
    { name: "pluginSetEnabled", call: () => pluginSetEnabled("a.b", false), command: "plugin_set_enabled", args: { pluginId: "a.b", enabled: false } },
    {
      name: "pluginGrantPermissions",
      call: () => pluginGrantPermissions("a.b", ["kv.read", "kv.write"]),
      command: "plugin_grant_permissions",
      args: { pluginId: "a.b", permissions: ["kv.read", "kv.write"] },
    },
    { name: "pluginUninstall", call: () => pluginUninstall("a.b", true), command: "plugin_uninstall", args: { pluginId: "a.b", keepData: true } },
    {
      name: "pluginRunCommand",
      call: () => pluginRunCommand("a.b", "go", [1, 2]),
      command: "plugin_run_command",
      args: { pluginId: "a.b", commandId: "go", assetIds: [1, 2] },
    },
    { name: "pluginContributions", call: () => pluginContributions(), command: "plugin_contributions" },
    { name: "pluginRunAll", call: () => pluginRunAll("a.b", "p1"), command: "plugin_run_all", args: { pluginId: "a.b", providerId: "p1" } },
    {
      name: "pluginExport",
      call: () => pluginExport("a.b", "csv", [3], "D:\\out.csv"),
      command: "plugin_export",
      args: { pluginId: "a.b", providerId: "csv", assetIds: [3], destPath: "D:\\out.csv" },
    },
    { name: "pluginTaskCounts", call: () => pluginTaskCounts(), command: "plugin_task_counts" },
    { name: "pluginRetryFailed", call: () => pluginRetryFailed("a.b"), command: "plugin_retry_failed", args: { pluginId: "a.b" } },
  ]

  it("T-ui-ipc-plugin-01 [Happy] each wrapper invokes the exact command with the exact camelCase args and returns the server value", async () => {
    // Arrange
    expect(cases).toHaveLength(12)
    for (const c of cases) {
      vi.mocked(invoke).mockReset()
      vi.mocked(invoke).mockResolvedValue(info)
      // Act
      const got = await c.call()
      // Assert
      expect(vi.mocked(invoke).mock.calls, c.name).toEqual([c.args ? [c.command, c.args] : [c.command]])
      expect(got, c.name).toBe(info)
    }
  })

  it("T-ui-ipc-plugin-01b [External] a rejected invoke propagates the CommandError unchanged for every wrapper", async () => {
    // Arrange
    const err: CommandError = { code: "plugin_permission_denied", message: "denied" }
    for (const c of cases) {
      vi.mocked(invoke).mockReset()
      vi.mocked(invoke).mockRejectedValueOnce(err)
      // Act + Assert
      await expect(c.call(), c.name).rejects.toBe(err)
    }
  })
})

describe("plugin ipc fixture contract", () => {
  it("T-ui-ipc-plugin-02 [Boundary] the shared fixture matches the TS types: key sets, enum values, non-empty lists", () => {
    // Arrange + Act + Assert
    expect(keysOf(fixture.pluginInfo)).toEqual(Object.keys(INFO_KEYS).sort())
    expect(keysOf(fixture.pluginInfoBroken)).toEqual(Object.keys(INFO_KEYS).sort())
    expect(keysOf(fixture.pluginPreview)).toEqual(Object.keys(PREVIEW_KEYS).sort())
    expect(keysOf(fixture.pluginContributions)).toEqual(["exportProviders", "menuItems"])
    expect(fixture.pluginInfo.commands.length).toBeGreaterThan(0)
    expect(fixture.pluginInfo.providers.length).toBeGreaterThan(0)
    expect(fixture.pluginContributions.menuItems.length).toBeGreaterThan(0)
    expect(fixture.pluginContributions.exportProviders.length).toBeGreaterThan(0)
    expect(fixture.taskCounts.length).toBeGreaterThan(0)
    for (const p of [...fixture.pluginInfo.providers, ...fixture.pluginPreview.providers]) {
      expect(keysOf(p)).toEqual(["assetKinds", "fileExtension", "id", "kind", "title"])
      expect(["metadata", "analysis", "export"]).toContain(p.kind)
    }
    expect(keysOf(fixture.pluginContributions.menuItems[0])).toEqual(["commandId", "pluginId", "title", "when"])
    expect(keysOf(fixture.pluginContributions.exportProviders[0])).toEqual(["assetKinds", "fileExtension", "pluginId", "providerId", "title"])
    expect(fixture.pluginContributions.menuItems.some((m) => m.when === null)).toBe(true)
    expect(keysOf(fixture.taskCounts[0])).toEqual(["failed", "pending", "pluginId"])
    expect(keysOf(fixture.commandResult)).toEqual(["message"])
    expect(fixture.commandResultNull.message).toBeNull()
    expect(keysOf(fixture.runAllResult)).toEqual(["started"])
    expect(keysOf(fixture.exportResult)).toEqual(["bytesWritten"])
    expect(keysOf(fixture.changedEvent)).toEqual(["pluginId", "reason"])
    expect(["ok", "broken", "incompatible"]).toContain(fixture.pluginInfoBroken.status)
    expect(["install", "upgrade", "reinstall", "downgrade"]).toContain(fixture.pluginPreview.change)
    expect(fixture.pluginInfo.unsigned).toBe(true)
    expect(fixture.pluginPreview.installed && keysOf(fixture.pluginPreview.installed)).toEqual(["granted", "version"])
    // 모든 권한 문자열이 알려진 권한이다
    for (const perm of [
      ...fixture.pluginInfo.permissions, ...fixture.pluginInfo.granted, ...fixture.pluginPreview.permissions,
      ...fixture.pluginPreview.addedPermissions, ...(fixture.pluginPreview.installed?.granted ?? []),
    ]) {
      expect(PLUGIN_PERMISSIONS).toContain(perm)
    }
  })

  it("T-ui-ipc-plugin-03 [Validation] every fixture error code is a known CommandError code that round-trips to errors.<code>", () => {
    // Arrange
    expect(fixture.errorCodes.length).toBe(6)
    for (const code of fixture.errorCodes) {
      const err: CommandError = { code, message: "m" }
      // Act + Assert
      expect(isCommandError(err), code).toBe(true)
      expect(errorKey(err)).toBe(`errors.${code}`)
    }
    // 알 수 없는 코드는 거부 → internal
    expect(isCommandError({ code: "plugin_unknown", message: "m" })).toBe(false)
    expect(errorKey({ code: "plugin_unknown", message: "m" })).toBe("errors.internal")
  })

  it("T-ui-ipc-plugin-04 [Happy] onPluginChanged subscribes to plugin://changed and forwards the payload; unlisten is returned", async () => {
    // Arrange
    const unlisten = vi.fn()
    let registered: ((e: { payload: PluginChanged }) => void) | undefined
    vi.mocked(listen).mockImplementation(async (_name, cb) => {
      registered = cb as typeof registered
      return unlisten
    })
    const handler = vi.fn()
    // Act
    const off = await onPluginChanged(handler)
    registered?.({ payload: fixture.changedEvent })
    // Assert
    expect(PLUGIN_CHANGED_EVENT).toBe(fixture.eventName)
    expect(vi.mocked(listen).mock.calls[0][0]).toBe("plugin://changed")
    expect(handler.mock.calls).toEqual([[fixture.changedEvent]])
    expect(off).toBe(unlisten)
    // 모든 변경 사유가 그대로 전달된다
    for (const reason of fixture.changeReasons) registered?.({ payload: { pluginId: "x", reason } })
    expect(handler).toHaveBeenCalledTimes(1 + fixture.changeReasons.length)
  })

  it("T-ui-ipc-plugin-05 [Validation] JobKind accepts plugin (mock job summary has a plugin counter) and fixture permissions equal PLUGIN_PERMISSIONS", async () => {
    // Arrange
    const kind: JobKind = "plugin"
    const api = createMockApi({ count: 5 })
    // Act
    const summary = await api.jobSummary()
    const paused = await api.pauseJobs(kind)
    // Assert
    expect(summary.counts.plugin).toEqual({ pending: 0, running: 0, failed: 0 })
    expect(paused.pausedKinds).toContain("plugin")
    expect([...PLUGIN_PERMISSIONS]).toEqual(fixture.permissions)
    expect(fixture.changeReasons).toHaveLength(7)
  })
})
