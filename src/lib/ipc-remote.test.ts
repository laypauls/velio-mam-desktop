// Categories: Happy (T-ui-ipc-remote-01) / Boundary (02 null) / External (03 reject passthrough) / Business rule (04 tauri api wiring, 05 mock rules, 06 secret write-only, 07 ses_queue rules) / Validation (08 unsafe dbPath)
import { invoke } from "@tauri-apps/api/core"
import { beforeEach, describe, expect, it, vi } from "vitest"
import { createMockApi } from "@/lib/api/mock"
import { createTauriApi } from "@/lib/api/tauri"
import type { CommandError, RemoteSource, RemoteSourceInput, RemoteTestResult } from "@/lib/ipc-contract"
import { createRemoteSource, deleteRemoteSource, listRemoteSources, testRemoteSource, updateRemoteSource } from "./ipc"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

const FP = "SHA256:Ig4rZ7Z7ET3zVOTrfedvfKbmxZCYcYRmG9eCLLjzATs"
const SECRET = "pw-never-leaks-ui"

const INPUT: RemoteSourceInput = {
  kind: "ddp_audit",
  name: "ddp-1",
  host: "10.0.0.5",
  port: 22,
  user: "audit",
  authKind: "password",
  secret: SECRET,
  hostKey: FP,
  logDir: null,
  dbPath: null,
  pollSec: 3,
  pathMap: [{ from: "/", to: "F:\\UHD" }],
}

/** Rust 가 돌려주는 모양(camelCase) — 비밀 없음 */
const VIEW: RemoteSource = {
  sourceId: 5,
  kind: "ddp_audit",
  name: "ddp-1",
  host: "10.0.0.5",
  port: 22,
  user: "audit",
  authKind: "password",
  hasSecret: true,
  hostKey: FP,
  logDir: null,
  dbPath: null,
  pollSec: 3,
  pathMap: [{ from: "/", to: "F:\\UHD" }],
  createdAt: 1,
  updatedAt: 1,
}

beforeEach(() => {
  vi.mocked(invoke).mockReset()
})

describe("ipc remote-source commands", () => {
  it("T-ui-ipc-remote-01 [Happy] wrappers use the Rust command and argument names", async () => {
    // Arrange
    const result: RemoteTestResult = { ok: true, errorKey: null, fingerprint: FP, fileCount: 3, rowCount: null, maxId: null }
    vi.mocked(invoke)
      .mockResolvedValueOnce([VIEW])
      .mockResolvedValueOnce(VIEW)
      .mockResolvedValueOnce(VIEW)
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce(result)
    // Act
    const list = await listRemoteSources()
    await createRemoteSource(INPUT)
    await updateRemoteSource(5, { ...INPUT, secret: null })
    await deleteRemoteSource(5)
    const tested = await testRemoteSource(5, INPUT)
    // Assert
    expect(list).toEqual([VIEW])
    expect(tested).toEqual(result)
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["list_remote_sources"],
      ["create_remote_source", { input: INPUT }],
      ["update_remote_source", { sourceId: 5, input: { ...INPUT, secret: null } }],
      ["delete_remote_source", { sourceId: 5 }],
      ["test_remote_source", { sourceId: 5, input: INPUT }],
    ])
  })

  it("T-ui-ipc-remote-02 [Boundary] a test before saving sends sourceId null and a missing secret is sent as null (keep the stored one)", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue({ ok: false, errorKey: "host_key_unconfirmed", fingerprint: FP, fileCount: null, rowCount: null, maxId: null })
    // Act
    await testRemoteSource(null, { ...INPUT, hostKey: null })
    await updateRemoteSource(5, { ...INPUT, secret: null })
    // Assert
    expect(vi.mocked(invoke).mock.calls[0]).toEqual(["test_remote_source", { sourceId: null, input: { ...INPUT, hostKey: null } }])
    expect(vi.mocked(invoke).mock.calls[1][1]).toMatchObject({ input: { secret: null } })
  })

  it("T-ui-ipc-remote-03 [External] a CommandError reject passes through unchanged and carries no secret", async () => {
    // Arrange
    const err: CommandError = { code: "invalid_input", message: "invalid input: pathMap" }
    vi.mocked(invoke).mockRejectedValue(err)
    // Act / Assert
    await expect(createRemoteSource(INPUT)).rejects.toEqual(err)
    expect(JSON.stringify(err)).not.toContain(SECRET)
  })

  it("T-ui-ipc-remote-04 [Business rule] the Tauri api routes the five methods to the wrappers", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue([])
    const api = createTauriApi()
    // Act
    await api.listRemoteSources()
    await api.createRemoteSource(INPUT)
    await api.updateRemoteSource(5, INPUT)
    await api.deleteRemoteSource(5)
    await api.testRemoteSource(null, INPUT)
    // Assert
    expect(vi.mocked(invoke).mock.calls.map((c) => c[0])).toEqual([
      "list_remote_sources",
      "create_remote_source",
      "update_remote_source",
      "delete_remote_source",
      "test_remote_source",
    ])
  })
})

describe("mock remote-source api", () => {
  const mock = () => createMockApi({ count: 5 })

  it("T-ui-ipc-remote-05 [Business rule] the mock follows the server rules: required secret, duplicate names, field-named validation, host key flow", async () => {
    // Arrange
    const api = mock()
    // Act
    const created = await api.createRemoteSource(INPUT)
    // Assert: 읽기 모델에 비밀이 없다
    expect(created).toMatchObject({ sourceId: 1, hasSecret: true, hostKey: FP })
    expect(JSON.stringify(created)).not.toContain(SECRET)
    expect(JSON.stringify(await api.listRemoteSources())).not.toContain(SECRET)
    await expect(api.createRemoteSource({ ...INPUT, secret: null })).rejects.toMatchObject({ code: "invalid_input", message: "invalid input: secret" })
    await expect(api.createRemoteSource({ ...INPUT, name: "DDP-1" })).rejects.toMatchObject({ code: "already_registered" })
    await expect(api.createRemoteSource({ ...INPUT, name: "b", port: 0 })).rejects.toMatchObject({ message: "invalid input: port" })
    await expect(api.createRemoteSource({ ...INPUT, name: "b", pathMap: [{ from: "/", to: "relative" }] })).rejects.toMatchObject({ message: "invalid input: pathMap" })
    await expect(api.createRemoteSource({ ...INPUT, name: "b", hostKey: "SHA256:short" })).rejects.toMatchObject({ message: "invalid input: hostKey" })
    // 지문 확인 흐름: 지문 없음 -> 지문만 돌려줌, 다른 지문 -> 거부, 같은 지문 -> 성공
    expect(await api.testRemoteSource(null, { ...INPUT, hostKey: null })).toMatchObject({ ok: false, errorKey: "host_key_unconfirmed", fingerprint: FP })
    expect(await api.testRemoteSource(1, { ...INPUT, secret: null, hostKey: `SHA256:${"b".repeat(43)}` })).toMatchObject({ ok: false, errorKey: "host_key_mismatch" })
    expect(await api.testRemoteSource(1, { ...INPUT, secret: null })).toMatchObject({ ok: true, fileCount: 3 })
  })

  it("T-ui-ipc-remote-06 [Business rule] update keeps the secret when omitted, delete of a missing source is not_found", async () => {
    // Arrange
    const api = mock()
    const { sourceId } = await api.createRemoteSource(INPUT)
    // Act
    const updated = await api.updateRemoteSource(sourceId, { ...INPUT, secret: null, port: 2200 })
    await api.deleteRemoteSource(sourceId)
    // Assert
    expect(updated).toMatchObject({ port: 2200, hasSecret: true })
    expect(await api.listRemoteSources()).toEqual([])
    await expect(api.deleteRemoteSource(sourceId)).rejects.toMatchObject({ code: "not_found" })
  })
})

describe("mock ses_queue connections", () => {
  const SES_INPUT: RemoteSourceInput = {
    ...INPUT,
    kind: "ses_queue",
    name: "ses-1",
    logDir: null,
    dbPath: "/srv/ses/events.db",
    pathMap: [{ from: "/data/incoming", to: "\\\\nas\\media" }],
  }
  const mock = () => createMockApi({ count: 5 })

  it("T-ui-ipc-remote-07 [Business rule] dbPath is required for ses_queue and forbidden for ddp_audit, the kind is fixed, and the test returns row count and max id", async () => {
    // Arrange
    const api = mock()
    // Act
    const ses = await api.createRemoteSource(SES_INPUT)
    // Assert
    expect(ses).toMatchObject({ kind: "ses_queue", dbPath: "/srv/ses/events.db", logDir: null, hasSecret: true })
    expect(JSON.stringify(ses)).not.toContain(SECRET)
    await expect(api.createRemoteSource({ ...SES_INPUT, name: "b", dbPath: null })).rejects.toMatchObject({ message: "invalid input: dbPath" })
    await expect(api.createRemoteSource({ ...SES_INPUT, name: "b", logDir: "/tmp/audit" })).rejects.toMatchObject({ message: "invalid input: logDir" })
    await expect(api.createRemoteSource({ ...INPUT, name: "c", dbPath: "/srv/ses/events.db" })).rejects.toMatchObject({ message: "invalid input: dbPath" })
    await expect(api.updateRemoteSource(ses.sourceId, { ...INPUT, name: "ses-1", secret: null })).rejects.toMatchObject({ message: "invalid input: kind" })
    expect(await api.testRemoteSource(ses.sourceId, { ...SES_INPUT, secret: null })).toMatchObject({ ok: true, rowCount: 3, maxId: 3, fileCount: null })
    expect(await api.testRemoteSource(null, { ...SES_INPUT, dbPath: "/srv/other.db" })).toMatchObject({ ok: false, errorKey: "db_unreadable", rowCount: null })
    expect(await api.testRemoteSource(null, { ...SES_INPUT, hostKey: null })).toMatchObject({ ok: false, errorKey: "host_key_unconfirmed", rowCount: null })
  })

  it("T-ui-ipc-remote-08 [Validation] unsafe dbPath values are rejected the way the Rust validation does", async () => {
    // Arrange
    const api = mock()
    const bad = ["relative.db", "/a;rm -rf /", "/a/'b", "/a/$(id)", "/a/`id`", "/a/b\nc", "/a/../b", "/a/b/", "/a|b", "/a\\b", "/a/*", "/" + "a".repeat(512)]
    // Act / Assert
    for (const dbPath of bad) {
      await expect(api.createRemoteSource({ ...SES_INPUT, dbPath })).rejects.toMatchObject({ code: "invalid_input", message: "invalid input: dbPath" })
    }
    expect(await api.listRemoteSources()).toEqual([])
    // 한글·공백이 든 경로는 허용
    await expect(api.createRemoteSource({ ...SES_INPUT, dbPath: "/var/lib/ses/큐 폴더/events.db" })).resolves.toMatchObject({ kind: "ses_queue" })
  })
})
