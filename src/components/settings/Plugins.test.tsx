// Categories: Happy (T-ui-plugin-01~03, 08, 10, 11, 13) / Boundary (04, 07) / Validation (05, 06) / Business rule (09, 12) / External (03b, 05b)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import i18n from "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import * as ipc from "@/lib/ipc-plugin"
import type { PluginChanged, PluginErrorCode, PluginInfo, PluginPreview, PluginTaskCount } from "@/lib/ipc-plugin"
import fixtureJson from "../../../src-tauri/src/plugin_ipc_fixture.json"
import { PluginsTab } from "./PluginsTab"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }))
vi.mock("@/lib/ipc-plugin", () => ({
  pluginList: vi.fn(),
  pluginTaskCounts: vi.fn(),
  pluginInspectDir: vi.fn(),
  pluginInstall: vi.fn(),
  pluginSetEnabled: vi.fn(),
  pluginUninstall: vi.fn(),
  pluginRunAll: vi.fn(),
  pluginRetryFailed: vi.fn(),
  onPluginChanged: vi.fn(),
}))

// 공유 픽스처(Rust 쪽 시험과 같은 JSON) — 모양이 계약과 같다
const fixture = fixtureJson as unknown as { pluginInfo: PluginInfo; pluginInfoBroken: PluginInfo; pluginPreview: PluginPreview; errorCodes: PluginErrorCode[] }
const DIR = "D:\\plugins\\exif"

const exif: PluginInfo = { ...fixture.pluginInfo, permissions: ["asset.read.metadata", "kv.write", "tag.write"], granted: ["asset.read.metadata", "kv.write"] }
const installPreview: PluginPreview = {
  ...fixture.pluginPreview,
  version: "1.0.0",
  installed: null,
  change: "install",
  addedPermissions: [],
  hasKeptData: false,
  permissions: ["asset.read.metadata", "kv.write", "tag.write"],
}
const upgradePreview: PluginPreview = { ...fixture.pluginPreview, change: "upgrade", addedPermissions: ["tag.write"], permissions: ["asset.read.metadata", "kv.write", "tag.write"] }

let plugins: PluginInfo[] = []
let counts: PluginTaskCount[] = []
let changedHandler: ((c: PluginChanged) => void) | null = null
let pick: string | null = DIR

function setup() {
  const api = createMockApi({ count: 10 })
  vi.spyOn(api, "pickDirectory").mockImplementation(() => Promise.resolve(pick))
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <ApiProvider api={api}>
        <PluginsTab />
      </ApiProvider>
    </QueryClientProvider>
  )
  return { user: userEvent.setup(), api }
}

beforeEach(() => {
  plugins = []
  counts = []
  changedHandler = null
  pick = DIR
  vi.mocked(ipc.pluginList).mockImplementation(() => Promise.resolve(plugins))
  vi.mocked(ipc.pluginTaskCounts).mockImplementation(() => Promise.resolve(counts))
  vi.mocked(ipc.onPluginChanged).mockImplementation((h) => {
    changedHandler = h
    return Promise.resolve(() => undefined)
  })
})

afterEach(() => vi.clearAllMocks())

describe("settings > plugins", () => {
  it("T-ui-plugin-01 [Happy] empty state shows title + description + the install action", async () => {
    setup()
    expect(await screen.findByRole("heading", { name: "설치된 플러그인이 없습니다" })).toBeInTheDocument()
    expect(screen.getByText("플러그인 폴더를 골라 설치하세요.")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "폴더에서 설치…" })).toBeEnabled()
    expect(screen.queryByRole("list", { name: "설치된 플러그인" })).not.toBeInTheDocument()
  })

  it("T-ui-plugin-02 [Happy] a list card shows title, version, unsigned badge, id, granted permissions in words, counts, retry and run-all", async () => {
    plugins = [exif, { ...exif, id: "com.example.plain", displayName: "Plain", providers: [], granted: [] }]
    counts = [{ pluginId: exif.id, pending: 12, failed: 1 }]
    setup()
    const list = await screen.findByRole("list", { name: "설치된 플러그인" })
    const cards = within(list).getAllByRole("listitem")
    expect(cards).toHaveLength(2)
    const card = cards[0]
    expect(within(card).getByRole("heading", { name: exif.displayName })).toBeInTheDocument()
    expect(within(card).getByText("v1.2.0")).toBeInTheDocument()
    expect(within(card).getByText("미서명")).toBeInTheDocument()
    expect(within(card).getByText(exif.id)).toBeInTheDocument()
    // 부여된 권한만, 사용자 말로 (요청만 하고 받지 못한 tag.write 는 제외)
    expect(within(card).getByText("권한: 자산 메타데이터 읽기 · 플러그인 데이터 저장")).toBeInTheDocument()
    expect(await within(card).findByText("대기 12 · 실패 1")).toBeInTheDocument()
    expect(within(card).getByRole("button", { name: "재시도" })).toBeEnabled()
    // 내보내기 제공자는 전체 실행 대상이 아니다 — metadata 제공자 1개
    expect(within(card).getAllByRole("button", { name: "전체 실행" })).toHaveLength(1)
    // 제공자가 없는 플러그인은 전체 실행 없음, 실패 0 이면 재시도 없음, 권한 없음 문구
    expect(within(cards[1]).queryByRole("button", { name: "전체 실행" })).not.toBeInTheDocument()
    expect(within(cards[1]).queryByRole("button", { name: "재시도" })).not.toBeInTheDocument()
    expect(within(cards[1]).getByText("권한: 없음")).toBeInTheDocument()
  })

  it("T-ui-plugin-03 [Happy] install: pick folder → inspect → consent dialog → install with the previewed id/version and every requested permission", async () => {
    vi.mocked(ipc.pluginInspectDir).mockResolvedValue(installPreview)
    vi.mocked(ipc.pluginInstall).mockImplementation(() => {
      plugins = [exif]
      return Promise.resolve(exif)
    })
    const { user } = setup()
    await user.click(await screen.findByRole("button", { name: "폴더에서 설치…" }))
    const dialog = await screen.findByRole("dialog", { name: "플러그인 설치: EXIF summary v1.0.0" })
    expect(ipc.pluginInspectDir).toHaveBeenCalledWith(DIR)
    expect(within(dialog).getByText("이 플러그인은 서명되지 않았습니다. 출처를 확인한 플러그인만 설치하세요.")).toBeInTheDocument()
    const requested = within(dialog).getByRole("region", { name: "요청 권한" })
    expect(within(requested).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["자산 메타데이터 읽기", "플러그인 데이터 저장", "태그 추가"])
    expect(within(dialog).getByText("원본 파일 수정·네트워크·프로그램 실행 권한은 제공되지 않습니다.")).toBeInTheDocument()
    expect(ipc.pluginInstall).not.toHaveBeenCalled() // 동의 전에는 설치하지 않는다
    await user.click(within(dialog).getByRole("button", { name: "설치" }))
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(ipc.pluginInstall).toHaveBeenCalledWith(DIR, installPreview.id, "1.0.0", ["asset.read.metadata", "kv.write", "tag.write"])
    expect(await screen.findByRole("list", { name: "설치된 플러그인" })).toBeInTheDocument()
    expect(toast.success).toHaveBeenCalledTimes(1)
  })

  it("T-ui-plugin-03b [External] install failure stays in the dialog as an inline alert (toast is secondary) and can be retried", async () => {
    vi.mocked(ipc.pluginInspectDir).mockResolvedValue(installPreview)
    vi.mocked(ipc.pluginInstall).mockRejectedValueOnce({ code: "plugin_incompatible", message: "x" })
    const { user } = setup()
    await user.click(await screen.findByRole("button", { name: "폴더에서 설치…" }))
    const dialog = await screen.findByRole("dialog")
    await user.click(within(dialog).getByRole("button", { name: "설치" }))
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("이 앱 버전과 호환되지 않는 플러그인입니다.")
    expect(screen.getByRole("dialog")).toBeInTheDocument()
    expect(toast.error).toHaveBeenCalledTimes(1)
    expect(within(dialog).getByRole("button", { name: "설치" })).toBeEnabled()
  })

  it("T-ui-plugin-04 [Boundary] cancelling the folder picker does nothing; cancelling the dialog never installs", async () => {
    vi.mocked(ipc.pluginInspectDir).mockResolvedValue(installPreview)
    const { user, api } = setup()
    pick = null
    await user.click(await screen.findByRole("button", { name: "폴더에서 설치…" }))
    await waitFor(() => expect(api.pickDirectory).toHaveBeenCalledTimes(1))
    expect(ipc.pluginInspectDir).not.toHaveBeenCalled()
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    pick = DIR
    await user.click(screen.getByRole("button", { name: "폴더에서 설치…" }))
    const dialog = await screen.findByRole("dialog")
    await user.click(within(dialog).getByRole("button", { name: "취소" }))
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(ipc.pluginInstall).not.toHaveBeenCalled()
  })

  it("T-ui-plugin-05 [Validation] a folder that fails inspection shows an inline alert, with no install button", async () => {
    vi.mocked(ipc.pluginInspectDir).mockRejectedValue({ code: "plugin_invalid", message: "bad manifest" })
    const { user } = setup()
    await user.click(await screen.findByRole("button", { name: "폴더에서 설치…" }))
    const dialog = await screen.findByRole("dialog", { name: "플러그인을 설치할 수 없습니다" })
    expect(within(dialog).getByRole("alert")).toHaveTextContent("플러그인 폴더나 설정 파일(manifest)이 올바르지 않습니다")
    expect(within(dialog).queryByRole("button", { name: "설치" })).not.toBeInTheDocument()
    expect(ipc.pluginInstall).not.toHaveBeenCalled()
  })

  it("T-ui-plugin-06 [Business rule] upgrade with added permissions highlights them and installs only after re-consent with the full set", async () => {
    plugins = [{ ...exif, permissions: ["asset.read.metadata", "kv.write"], granted: ["asset.read.metadata", "kv.write"] }]
    vi.mocked(ipc.pluginInspectDir).mockResolvedValue(upgradePreview)
    vi.mocked(ipc.pluginInstall).mockResolvedValue(exif)
    const { user } = setup()
    await user.click(await screen.findByRole("button", { name: "폴더에서 설치…" }))
    const dialog = await screen.findByRole("dialog", { name: "플러그인 업그레이드: EXIF summary v1.3.0" })
    const added = within(dialog).getByRole("region", { name: "새로 요청하는 권한" })
    expect(within(added).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["태그 추가"])
    const kept = within(dialog).getByRole("region", { name: "기존 권한" })
    expect(within(kept).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["자산 메타데이터 읽기", "플러그인 데이터 저장"])
    expect(ipc.pluginInstall).not.toHaveBeenCalled()
    await user.click(within(dialog).getByRole("button", { name: "설치" }))
    await waitFor(() => expect(ipc.pluginInstall).toHaveBeenCalledWith(DIR, upgradePreview.id, "1.3.0", ["asset.read.metadata", "kv.write", "tag.write"]))
  })

  it("T-ui-plugin-07 [Boundary] a plugin with no requested permissions says so", async () => {
    vi.mocked(ipc.pluginInspectDir).mockResolvedValue({ ...installPreview, permissions: [] })
    const { user } = setup()
    await user.click(await screen.findByRole("button", { name: "폴더에서 설치…" }))
    const dialog = await screen.findByRole("dialog")
    expect(within(dialog).getByText("이 플러그인은 권한을 요청하지 않습니다.")).toBeInTheDocument()
    expect(within(dialog).getByRole("button", { name: "설치" })).toBeEnabled()
  })

  it("T-ui-plugin-07b [Business rule] same-ID overwrite warning shows for reinstall/upgrade/downgrade and for kept data, never for a brand-new ID", async () => {
    const warning = "같은 ID 의 기존 플러그인을 덮어씁니다 — 다른 작성자의 패키지일 수 있고, 기존 플러그인 데이터에 접근합니다."
    const cases: { label: string; preview: PluginPreview; shown: boolean }[] = [
      { label: "new id", preview: installPreview, shown: false },
      { label: "upgrade", preview: upgradePreview, shown: true },
      { label: "reinstall", preview: { ...upgradePreview, change: "reinstall", addedPermissions: [] }, shown: true },
      { label: "downgrade", preview: { ...upgradePreview, change: "downgrade", addedPermissions: [] }, shown: true },
      { label: "kept data only", preview: { ...installPreview, hasKeptData: true }, shown: true },
    ]
    for (const c of cases) {
      vi.mocked(ipc.pluginInspectDir).mockResolvedValue(c.preview)
      const { user } = setup()
      await user.click(await screen.findByRole("button", { name: "폴더에서 설치…" }))
      const dialog = await screen.findByRole("dialog")
      if (c.shown) expect(within(dialog).getByText(warning), c.label).toBeInTheDocument()
      else expect(within(dialog).queryByText(warning), c.label).not.toBeInTheDocument()
      expect(ipc.pluginInstall).not.toHaveBeenCalled() // 경고만 — 동의 전에는 설치하지 않는다
      cleanup()
    }
  })

  it("T-ui-plugin-08 [Happy] the enable switch turns the plugin off and back on", async () => {
    plugins = [exif]
    vi.mocked(ipc.pluginSetEnabled).mockImplementation((id, enabled) => {
      plugins = plugins.map((p) => (p.id === id ? { ...p, enabled } : p))
      return Promise.resolve(plugins[0])
    })
    const { user } = setup()
    const sw = await screen.findByRole("switch", { name: "EXIF summary 사용" })
    expect(sw).toBeChecked()
    await user.click(sw)
    await waitFor(() => expect(screen.getByRole("switch", { name: "EXIF summary 사용" })).not.toBeChecked())
    expect(ipc.pluginSetEnabled).toHaveBeenLastCalledWith(exif.id, false)
    await user.click(screen.getByRole("switch", { name: "EXIF summary 사용" }))
    await waitFor(() => expect(screen.getByRole("switch", { name: "EXIF summary 사용" })).toBeChecked())
    expect(ipc.pluginSetEnabled).toHaveBeenLastCalledWith(exif.id, true)
  })

  it("T-ui-plugin-09 [Business rule] uninstall asks first, defaults to deleting data, states the restore note, and sends keepData", async () => {
    plugins = [exif]
    vi.mocked(ipc.pluginUninstall).mockImplementation(() => {
      plugins = []
      return Promise.resolve()
    })
    const { user } = setup()
    // keepData = false (기본)
    await user.click(await screen.findByRole("button", { name: "제거" }))
    let dialog = await screen.findByRole("alertdialog", { name: "플러그인 제거: EXIF summary" })
    expect(within(dialog).getByRole("checkbox", { name: "플러그인 데이터 보존" })).not.toBeChecked()
    expect(within(dialog).getByText("제거할 때 남겨 둔 데이터도 이후 백업에서 DB를 복원하면 삭제됩니다.")).toBeInTheDocument()
    expect(ipc.pluginUninstall).not.toHaveBeenCalled()
    await user.click(within(dialog).getByRole("button", { name: "제거" }))
    await waitFor(() => expect(ipc.pluginUninstall).toHaveBeenCalledWith(exif.id, false))
    expect(await screen.findByRole("heading", { name: "설치된 플러그인이 없습니다" })).toBeInTheDocument()

    // keepData = true
    plugins = [exif]
    await act(async () => changedHandler?.({ pluginId: exif.id, reason: "installed" }))
    await user.click(await screen.findByRole("button", { name: "제거" }))
    dialog = await screen.findByRole("alertdialog")
    await user.click(within(dialog).getByRole("checkbox", { name: "플러그인 데이터 보존" }))
    expect(within(dialog).getByRole("checkbox", { name: "플러그인 데이터 보존" })).toBeChecked()
    expect(within(dialog).getByText("제거할 때 남겨 둔 데이터도 이후 백업에서 DB를 복원하면 삭제됩니다.")).toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "제거" }))
    await waitFor(() => expect(ipc.pluginUninstall).toHaveBeenLastCalledWith(exif.id, true))
  })

  it("T-ui-plugin-09b [External] uninstall failure stays in the dialog as an inline alert", async () => {
    plugins = [exif]
    vi.mocked(ipc.pluginUninstall).mockRejectedValue({ code: "plugin_error", message: "x" })
    const { user } = setup()
    await user.click(await screen.findByRole("button", { name: "제거" }))
    const dialog = await screen.findByRole("alertdialog")
    await user.click(within(dialog).getByRole("button", { name: "제거" }))
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("플러그인을 실행하는 중 오류가 발생했습니다.")
  })

  it("T-ui-plugin-10 [Business rule] run-all is disabled while the plugin is disabled, and calls pluginRunAll for the provider when enabled", async () => {
    plugins = [{ ...exif, enabled: false }]
    vi.mocked(ipc.pluginRunAll).mockResolvedValue({ started: true })
    const { user } = setup()
    const runAll = await screen.findByRole("button", { name: "전체 실행" })
    expect(runAll).toBeDisabled()
    // 켜짐 → 활성
    plugins = [exif]
    await act(async () => changedHandler?.({ pluginId: exif.id, reason: "enabled" }))
    await waitFor(() => expect(screen.getByRole("button", { name: "전체 실행" })).toBeEnabled())
    await user.click(screen.getByRole("button", { name: "전체 실행" }))
    await waitFor(() => expect(ipc.pluginRunAll).toHaveBeenCalledWith(exif.id, "exif"))
    expect(toast.success).toHaveBeenCalledWith("모든 자산에 대한 실행을 대기열에 올렸습니다.")
  })

  it("T-ui-plugin-11 [Happy] retry appears only with failed tasks and re-queues them", async () => {
    plugins = [exif]
    counts = [{ pluginId: exif.id, pending: 0, failed: 3 }]
    vi.mocked(ipc.pluginRetryFailed).mockImplementation(() => {
      counts = [{ pluginId: exif.id, pending: 3, failed: 0 }]
      return Promise.resolve(3)
    })
    const { user } = setup()
    await user.click(await screen.findByRole("button", { name: "재시도" }))
    await waitFor(() => expect(ipc.pluginRetryFailed).toHaveBeenCalledWith(exif.id))
    expect(toast.success).toHaveBeenCalledWith("실패한 작업 3건을 다시 대기열에 올렸습니다.")
    expect(await screen.findByText("대기 3 · 실패 0")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "재시도" })).not.toBeInTheDocument()
  })

  it("T-ui-plugin-12 [Business rule] a broken plugin shows its state, cannot be toggled or run, but can be removed", async () => {
    plugins = [{ ...fixture.pluginInfoBroken, providers: [{ ...exif.providers[0] }] }]
    setup()
    const card = await screen.findByRole("listitem")
    expect(within(card).getByText("손상됨")).toBeInTheDocument()
    expect(within(card).getByRole("switch")).toBeDisabled()
    expect(within(card).getByRole("button", { name: "전체 실행" })).toBeDisabled()
    expect(within(card).getByRole("button", { name: "제거" })).toBeEnabled()
  })

  it("T-ui-plugin-13 [Happy] plugin://changed refreshes the list (auto-disable shows without reopening)", async () => {
    plugins = [exif]
    setup()
    expect(await screen.findByRole("switch", { name: "EXIF summary 사용" })).toBeChecked()
    plugins = [{ ...exif, enabled: false }]
    await act(async () => changedHandler?.({ pluginId: exif.id, reason: "autoDisabled" }))
    await waitFor(() => expect(screen.getByRole("switch", { name: "EXIF summary 사용" })).not.toBeChecked())
  })

  it("T-ui-plugin-14 [Validation] every plugin error code has its own Korean message (no fallback)", () => {
    expect(fixture.errorCodes.length).toBeGreaterThanOrEqual(6)
    for (const code of fixture.errorCodes) {
      expect(i18n.exists(`errors.${code}`), code).toBe(true)
      expect(i18n.t(`errors.${code}`)).not.toBe(i18n.t("errors.internal"))
    }
  })
})
