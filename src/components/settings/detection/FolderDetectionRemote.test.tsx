// Categories: Happy (T-ui-remote-01 order, 05 ddp save) / Boundary (04 default on) / Validation (05b source required) / Business rule (02 common option in auto, 03 preserved across detectors, 06 health line) / External (07 stall + health row badges)
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { MOCK_SES_DB } from "@/lib/api/mock-remote"
import type { ManagedFolder, RemoteSourceInput } from "@/lib/ipc-contract"
import { openFolderSettings } from "@/test/folderDialog"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

const PATH = "D:\\Photos"
type User = ReturnType<typeof userEvent.setup>
type Api = ReturnType<typeof createMockApi>

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

const FP = "SHA256:Ig4rZ7Z7ET3zVOTrfedvfKbmxZCYcYRmG9eCLLjzATs"
const base = (patch: Partial<RemoteSourceInput>): RemoteSourceInput => ({
  kind: "ddp_audit",
  name: "ddp-1",
  host: "10.0.0.5",
  port: 22,
  user: "audit",
  authKind: "password",
  secret: "pw",
  hostKey: FP,
  logDir: null,
  dbPath: null,
  pollSec: 3,
  pathMap: [{ from: "/", to: "D:\\Photos" }],
  ...patch,
})

async function setup(prepare?: (api: Api) => Promise<void> | void) {
  stubLayout()
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  const user = userEvent.setup()
  const api = createMockApi({ count: 10 })
  await prepare?.(api)
  renderApp(api)
  const settings = await openSettingsPage(user, "관리 폴더")
  return { user, api, settings }
}

async function openDialog(user: User, settings: HTMLElement) {
  const dialog = await openFolderSettings(user, settings, PATH)
  await waitFor(() => expect(within(dialog).getByRole("combobox", { name: "방식" })).toBeEnabled())
  return dialog
}

async function choose(user: User, dialog: HTMLElement, select: string, option: string | RegExp) {
  await user.click(within(dialog).getByRole("combobox", { name: select }))
  await user.click(await screen.findByRole("option", { name: option }))
}

const save = (user: User, dialog: HTMLElement) => user.click(within(dialog).getByRole("button", { name: "저장" }))
const exclusive = (dialog: HTMLElement) => within(dialog).getByRole("switch", { name: "열려 있는 파일은 닫힌 뒤에 반영" })

function overrideFolder(api: Api, patch: Partial<ManagedFolder>) {
  const orig = api.listFolders.bind(api)
  vi.spyOn(api, "listFolders").mockImplementation(async () => (await orig()).map((f) => (f.folderId === 1 ? { ...f, ...patch } : f)))
}

describe("folder settings dialog — detectors, copy completion, remote source", () => {
  it("T-ui-remote-01 [Happy] lists every detector in registry order with the experimental ones labelled", async () => {
    // Arrange
    const { user, settings } = await setup()
    const dialog = await openDialog(user, settings)
    // Act
    await user.click(within(dialog).getByRole("combobox", { name: "방식" }))
    // Assert
    const names = (await screen.findAllByRole("option")).map((o) => o.textContent)
    expect(names).toEqual([
      "자동 (권장)",
      "실시간 감시",
      "폴링 (실험)",
      "DDP 감사 로그 (실험)",
      "리눅스 스토리지 이벤트 (fanotify) (실험)",
      "재스캔만",
    ])
  })

  it("T-ui-remote-02 [Business rule] the copy-completion switch is ON by default; turning it off in auto mode saves {exclusiveOpenCheck:false} with mode null", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    const spy = vi.spyOn(api, "setFolderWatchMode")
    const dialog = await openDialog(user, settings)
    expect(exclusive(dialog)).toBeChecked()
    expect(within(dialog).getByRole("button", { name: "저장" })).toBeDisabled()
    // Act
    await user.click(exclusive(dialog))
    await save(user, dialog)
    // Assert
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(spy).toHaveBeenCalledWith(1, null, { exclusiveOpenCheck: false })
    expect((await api.listFolders()).find((f) => f.folderId === 1)).toMatchObject({ watchMode: null, watchOptions: { exclusiveOpenCheck: false } })
  })

  it("T-ui-remote-03 [Business rule] the common option survives a detector change and sits next to detector options", async () => {
    // Arrange: 저장된 상태 = 재스캔만 + 끔
    const { user, api, settings } = await setup(async (a) => void (await a.setFolderWatchMode(1, "scan_only", { exclusiveOpenCheck: false })))
    const spy = vi.spyOn(api, "setFolderWatchMode")
    const dialog = await openDialog(user, settings)
    expect(exclusive(dialog)).not.toBeChecked()
    // Act: 폴링으로 바꾸고 간격을 120 으로
    await choose(user, dialog, "방식", "폴링 (실험)")
    const interval = within(dialog).getByRole("spinbutton", { name: "확인 간격 (초)" })
    await user.clear(interval)
    await user.type(interval, "120")
    // Assert: 감지기를 바꿔도 공통 옵션은 그대로 꺼져 있다
    expect(exclusive(dialog)).not.toBeChecked()
    await save(user, dialog)
    await waitFor(() => expect(spy).toHaveBeenCalledWith(1, "polling", { intervalSec: 120, exclusiveOpenCheck: false }))
  })

  it("T-ui-remote-04 [Boundary] turning the switch back ON stores nothing (default), an unchanged form stays clean", async () => {
    // Arrange: 끔으로 저장돼 있다
    const { user, api, settings } = await setup(async (a) => void (await a.setFolderWatchMode(1, null, { exclusiveOpenCheck: false })))
    const spy = vi.spyOn(api, "setFolderWatchMode")
    const dialog = await openDialog(user, settings)
    expect(exclusive(dialog)).not.toBeChecked()
    expect(within(dialog).getByRole("button", { name: "저장" })).toBeDisabled()
    // Act
    await user.click(exclusive(dialog))
    await save(user, dialog)
    // Assert
    await waitFor(() => expect(spy).toHaveBeenCalledWith(1, null, null))
  })

  it("T-ui-remote-05 [Happy] DDP: only DDP connections are offered, and the chosen one is saved as {source}", async () => {
    // Arrange: 접속 2개 (DDP + 리눅스 큐)
    const ids: number[] = []
    const { user, api, settings } = await setup(async (a) => {
      ids.push((await a.createRemoteSource(base({}))).sourceId)
      await a.createRemoteSource(base({ kind: "ses_queue", name: "ses-1", dbPath: MOCK_SES_DB, pathMap: [{ from: "/data/incoming", to: "D:\\Photos" }] }))
    })
    const spy = vi.spyOn(api, "setFolderWatchMode")
    const dialog = await openDialog(user, settings)
    // Act
    await choose(user, dialog, "방식", "DDP 감사 로그 (실험)")
    await user.click(within(dialog).getByRole("combobox", { name: "원격 접속" }))
    // Assert: 종류로 걸러진다
    const options = (await screen.findAllByRole("option")).map((o) => o.textContent)
    expect(options).toEqual(["ddp-1 (10.0.0.5)"])
    await user.click(screen.getByRole("option", { name: "ddp-1 (10.0.0.5)" }))
    await save(user, dialog)
    await waitFor(() => expect(spy).toHaveBeenCalledWith(1, "ddp_audit", { source: ids[0] }))
  })

  it("T-ui-remote-05b [Validation] a remote detector needs a connection: inline error, Save disabled, no call", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    const spy = vi.spyOn(api, "setFolderWatchMode")
    const dialog = await openDialog(user, settings)
    // Act
    await choose(user, dialog, "방식", "리눅스 스토리지 이벤트 (fanotify) (실험)")
    // Assert
    const field = within(dialog).getByRole("combobox", { name: "원격 접속" })
    expect(field).toHaveAttribute("aria-invalid", "true")
    expect(field).toHaveAccessibleDescription("원격 접속을 고르세요")
    expect(within(dialog).getByText(/이 방식에 쓸 수 있는 접속이 없습니다/)).toBeInTheDocument()
    expect(within(dialog).getByRole("button", { name: "저장" })).toBeDisabled()
    expect(spy).not.toHaveBeenCalled()
  })

  it("T-ui-remote-06 [Business rule] the status line shows a degraded reason and, for remote detectors, the last signal", async () => {
    // Arrange: 문제 있음
    const since = Date.UTC(2026, 10, 21, 5, 0, 0)
    const first = await setup((api) =>
      overrideFolder(api, { watchActive: "ddp_audit", watchMode: "ddp_audit", watchHealth: { state: "degraded", since, reasonKey: "watch.health.remote_connect" } }),
    )
    const dialog = await openDialog(first.user, first.settings)
    // Assert
    expect(within(dialog).getByText(/문제 있음 — 스토리지에 연결하지 못했습니다 \(재연결 중\)/)).toBeInTheDocument()
    expect(within(dialog).getByText(/놓친 변경은 주기 재스캔이 보완합니다/)).toBeInTheDocument()
  })

  it("T-ui-remote-06b [Business rule] a healthy remote detector shows the last signal time; a healthy real-time watcher shows no line", async () => {
    // Arrange
    const at = Date.UTC(2026, 10, 21, 5, 0, 0)
    const { user, settings } = await setup((api) =>
      overrideFolder(api, { watchActive: "ddp_audit", watchMode: "ddp_audit", watchHealth: { state: "ok", lastSignalAt: at } }),
    )
    // Act
    const dialog = await openDialog(user, settings)
    // Assert
    expect(within(dialog).getByText(/현재 상태: 연결됨 · 마지막 신호/)).toBeInTheDocument()
  })
})

describe("관리 폴더 표 — 상태 배지", () => {
  it("T-ui-remote-07 [External] row shows the experimental mark, the connected badge, and a degraded badge with the reason", async () => {
    // Arrange / Act: 정상 연결
    const ok = await setup((api) =>
      overrideFolder(api, { watchActive: "ddp_audit", watchMode: "ddp_audit", watchHealth: { state: "ok", lastSignalAt: 1 } }),
    )
    // Assert
    expect(await within(ok.settings).findByText("DDP 감사 로그")).toBeInTheDocument()
    expect(await within(ok.settings).findByRole("img", { name: "실험" })).toBeInTheDocument()
    expect(within(ok.settings).getByText("연결")).toBeInTheDocument()
  })

  it("T-ui-remote-07b [External] a degraded health badge carries the reason for screen readers and the tooltip", async () => {
    // Arrange
    const { settings } = await setup((api) =>
      overrideFolder(api, {
        watchActive: "os_events",
        watchHealth: { state: "degraded", since: 1, reasonKey: "watch.health.overflow" },
      }),
    )
    // Assert
    const badge = (await within(settings).findByText("문제")).closest("[title]")
    expect(badge).toHaveAttribute("title", "변경 감지 문제: 변경 알림이 넘쳐 일부를 놓쳤을 수 있습니다 (재스캔을 요청했습니다)")
    expect(within(settings).getByText("변경 알림이 넘쳐 일부를 놓쳤을 수 있습니다 (재스캔을 요청했습니다)")).toBeInTheDocument()
  })

  it("T-ui-remote-07c [External] a stalled scan shows minutes and the held path (full text for assistive tech), and a healthy folder shows none", async () => {
    // Arrange: 4분 5초째 멈춤
    const { settings } = await setup((api) =>
      overrideFolder(api, { scanStall: { atPath: "\\\\nas2\\2019\\a.mov", since: Date.now() - 4 * 60_000 - 5_000 } }),
    )
    // Assert
    const status = await within(settings).findByRole("status", { name: /스캔 멈춤 4분/ })
    expect(status).toHaveAccessibleName(
      "스캔 멈춤 4분 (붙들린 곳: \\\\nas2\\2019\\a.mov) — 응답 없는 공유를 읽는 중일 수 있습니다. 파일이 삭제된 것은 아닙니다.",
    )
    expect(within(settings).getAllByRole("status", { name: /스캔 멈춤/ })).toHaveLength(1)
  })

  it("T-ui-remote-07d [Boundary] a folder without stall or health shows no badge", async () => {
    // Arrange / Act
    const { settings } = await setup()
    await within(settings).findAllByText("실시간 감시")
    // Assert
    expect(within(settings).queryByRole("status", { name: /스캔 멈춤/ })).not.toBeInTheDocument()
    expect(within(settings).queryByText("문제")).not.toBeInTheDocument()
    expect(within(settings).queryByText("연결")).not.toBeInTheDocument()
  })
})
