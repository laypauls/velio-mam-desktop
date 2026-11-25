// Categories: Happy (T-ui-pmenu-10 run + toast, -17 viewer) / Business rule (T-ui-pmenu-11 existing menu unchanged, -12 empty, -13 when hides, -14 offline/multi context) / External (T-ui-pmenu-15 failure toast, -16 null message)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary, SettingsPatch } from "@/lib/ipc-contract"
import type { PluginContributions } from "@/lib/ipc-plugin"
import { resetAllStores } from "@/stores"
import { AssetContextMenu } from "./AssetContextMenu"
import { useSelectionStore } from "@/stores"

const invokeMock = vi.hoisted(() => vi.fn())
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }))
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn(), info: vi.fn(), warning: vi.fn() } }))

const PLUGIN = "com.example.exif-summary"
const make = (assetId: number, over: Partial<AssetSummary> = {}): AssetSummary => ({
  assetId,
  fileName: `f${assetId}.jpg`,
  mediaType: "image",
  availability: "online",
  width: 100,
  height: 100,
  durationMs: null,
  fileSize: 1,
  capturedAt: null,
  modifiedAt: 0,
  favorite: false,
  rating: 0,
  thumbState: "pending",
  aiState: "none",
  title: null,
  revision: 1,
  ...over,
})

let contributions: PluginContributions | Error
let runResult: { message: string | null } | { code: string }
let exportError: { code: string } | null

beforeEach(() => {
  resetAllStores()
  contributions = { menuItems: [], exportProviders: [] }
  runResult = { message: "done" }
  exportError = null
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === "plugin_contributions") {
      if (contributions instanceof Error) throw contributions
      return contributions
    }
    if (cmd === "plugin_run_command") {
      if ("code" in runResult) throw runResult // Rust CommandError 모양: { code, ... }
      return runResult
    }
    if (cmd === "plugin_export") {
      if (exportError !== null) throw exportError
      return { bytesWritten: 2048 }
    }
    throw new Error(`unexpected invoke: ${cmd}`)
  })
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.clearAllMocks()
})

const item = (commandId: string, title: string, when: string | null) => ({ pluginId: PLUGIN, commandId, title, when })

async function openMenu(assets: AssetSummary[], settings: SettingsPatch = {}, select: number[] = []) {
  const api = createMockApi({ count: 10 })
  await api.updateSettings(settings)
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const byId = new Map(assets.map((a) => [a.assetId, a]))
  render(
    <QueryClientProvider client={queryClient}>
      <ApiProvider api={api}>
        <AssetContextMenu lookup={(id) => byId.get(id)}>
          {assets.map((a) => (
            <div key={a.assetId} data-asset-id={a.assetId}>
              {a.fileName}
            </div>
          ))}
        </AssetContextMenu>
      </ApiProvider>
    </QueryClientProvider>
  )
  if (select.length > 0) useSelectionStore.getState().selectAll(select)
  // 설정·기여 조회가 끝난 뒤 메뉴를 연다 (둘 다 한 번 받아 캐시 — 메뉴를 여는 시점에는 이미 있다)
  await queryClient.fetchQuery({ queryKey: ["settings"], queryFn: () => api.getSettings(), staleTime: Infinity })
  await queryClient.fetchQuery({ queryKey: ["pluginContributions"], queryFn: () => invokeMock("plugin_contributions"), staleTime: Infinity }).catch(() => undefined)
  fireEvent.contextMenu(screen.getByText(assets[0].fileName))
  await screen.findAllByRole("menuitem")
  return { user: userEvent.setup(), queryClient, api }
}

const menuNames = () => screen.getAllByRole("menuitem").map((e) => e.textContent ?? "")

describe("plugin command slot", () => {
  it("T-ui-pmenu-10 [Happy] a command whose `when` is true is listed before 내보내기, and selecting it runs it on the selection and toasts the plugin message", async () => {
    // Arrange
    contributions = { menuItems: [item("summarize", "EXIF 요약 만들기", "selection.count >= 1 && asset.kind == 'image'")], exportProviders: [] }
    const { user } = await openMenu([make(1)])
    const names = menuNames()
    // Assert order: slot item immediately before export
    expect(names.indexOf("EXIF 요약 만들기")).toBe(names.indexOf("내보내기") - 1)
    // Act
    await user.click(screen.getByRole("menuitem", { name: "EXIF 요약 만들기" }))
    // Assert
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("done"))
    expect(invokeMock).toHaveBeenCalledWith("plugin_run_command", { pluginId: PLUGIN, commandId: "summarize", assetIds: [1] })
  })

  it("T-ui-pmenu-11 [Business rule] with no contributions the menu keeps every existing item, order and disabled state (settings ON/OFF, offline asset)", async () => {
    // Arrange / Act
    await openMenu([make(1, { availability: "offline" })], { titleEditEnabled: true, originalActionsEnabled: true })
    const on = menuNames()
    const disabledOn = screen.getAllByRole("menuitem").filter((e) => e.getAttribute("aria-disabled") === "true").map((e) => e.textContent)
    // Assert
    expect(on.indexOf("제목 변경…")).toBeGreaterThan(-1)
    expect(on.indexOf("내보내기")).toBeLessThan(on.findIndex((n) => n.startsWith("이동")))
    expect(on.some((n) => n.startsWith("삭제"))).toBe(true)
    expect(disabledOn.some((n) => n?.startsWith("미리보기"))).toBe(true)
    expect(disabledOn.some((n) => n?.startsWith("내보내기"))).toBe(false)
    expect(screen.queryAllByRole("separator")).toHaveLength(3) // 제목 변경 뒤 · 태그 뒤 · 내보내기 뒤(원본 작업) — 슬롯 구분선 없음
  })

  it("T-ui-pmenu-12 [Business rule] settings OFF: no title-edit / move / delete, and still no plugin group or extra separator when contributions are empty", async () => {
    // Arrange / Act
    await openMenu([make(1)], { titleEditEnabled: false, originalActionsEnabled: false })
    const names = menuNames()
    // Assert
    expect(names).not.toContain("제목 변경…")
    expect(names.some((n) => n.startsWith("이동") || n.startsWith("삭제"))).toBe(false)
    expect(screen.queryAllByRole("separator")).toHaveLength(2)
    expect(screen.queryByText("EXIF 요약 만들기")).toBeNull()
  })

  it("T-ui-pmenu-13 [Business rule] a command whose `when` is false is hidden; when every command is hidden the group and its separators disappear", async () => {
    // Arrange
    contributions = { menuItems: [item("v", "영상 전용", "asset.kind == 'video'")], exportProviders: [] }
    // Act
    await openMenu([make(1)])
    // Assert
    expect(screen.queryByRole("menuitem", { name: "영상 전용" })).toBeNull()
    expect(screen.queryAllByRole("separator")).toHaveLength(2)
  })

  it("T-ui-pmenu-14 [Business rule] context: offline asset sets asset.offline; selection.count follows the multi-selection", async () => {
    // Arrange
    contributions = {
      menuItems: [
        item("off", "오프라인일 때", "asset.offline"),
        item("on", "온라인일 때", "!asset.offline"),
        item("many", "여러 개", "selection.count > 1"),
      ],
      exportProviders: [],
    }
    // Act
    await openMenu([make(1, { availability: "missing" })])
    // Assert
    expect(screen.getByRole("menuitem", { name: "오프라인일 때" })).toBeInTheDocument()
    expect(screen.queryByRole("menuitem", { name: "온라인일 때" })).toBeNull()
    expect(screen.queryByRole("menuitem", { name: "여러 개" })).toBeNull()
  })

  it("T-ui-pmenu-15 [External] a failed command toasts the mapped error message, never the raw code", async () => {
    // Arrange
    contributions = { menuItems: [item("summarize", "EXIF 요약 만들기", null)], exportProviders: [] }
    runResult = { code: "plugin_timeout" }
    const { user } = await openMenu([make(1)])
    // Act
    await user.click(screen.getByRole("menuitem", { name: "EXIF 요약 만들기" }))
    // Assert
    await waitFor(() => expect(toast.error).toHaveBeenCalledTimes(1))
    const message = vi.mocked(toast.error).mock.calls[0][0]
    expect(message).not.toContain("plugin_timeout")
    expect(message).not.toContain("errors.")
    expect(toast.success).not.toHaveBeenCalled()
  })

  it("T-ui-pmenu-16 [Boundary] a null result message toasts the generic success text", async () => {
    // Arrange
    contributions = { menuItems: [item("summarize", "EXIF 요약 만들기", null)], exportProviders: [] }
    runResult = { message: null }
    const { user } = await openMenu([make(1)])
    // Act
    await user.click(screen.getByRole("menuitem", { name: "EXIF 요약 만들기" }))
    // Assert
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("EXIF 요약 만들기 실행을 마쳤습니다."))
  })

  it("T-ui-pmenu-17 [External] when the contribution query fails the menu still shows the existing items without the group", async () => {
    // Arrange
    contributions = new Error("ipc down")
    // Act
    await openMenu([make(1)])
    // Assert
    expect(screen.getByRole("menuitem", { name: /내보내기/ })).toBeInTheDocument()
    expect(screen.queryAllByRole("separator")).toHaveLength(2)
  })
})

describe("plugin command slot — multi selection", () => {
  it("T-ui-pmenu-18 [Business rule] selection.count counts the whole multi-selection and the command receives every selected id", async () => {
    // Arrange
    contributions = { menuItems: [item("many", "여러 개", "selection.count > 1"), item("one", "하나만", "selection.count == 1")], exportProviders: [] }
    const { user } = await openMenu([make(1), make(2)], {}, [1, 2])
    // Assert
    expect(screen.queryByRole("menuitem", { name: "하나만" })).toBeNull()
    // Act
    await user.click(screen.getByRole("menuitem", { name: "여러 개" }))
    // Assert
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("plugin_run_command", { pluginId: PLUGIN, commandId: "many", assetIds: [1, 2] }))
  })
})

const provider = { pluginId: PLUGIN, providerId: "csv", title: "CSV 내보내기", fileExtension: "csv", assetKinds: ["image"] }

describe("plugin export providers", () => {
  it("T-ui-pmenu-19 [Happy] a provider is listed in the slot; picking a path runs pluginExport and toasts the size", async () => {
    // Arrange
    contributions = { menuItems: [], exportProviders: [provider] }
    const { user, api } = await openMenu([make(1)])
    const pick = vi.spyOn(api, "pickSaveFile").mockResolvedValue("D:\out\export.csv")
    // Act
    await user.click(screen.getByRole("menuitem", { name: "CSV 내보내기…" }))
    // Assert
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("plugin_export", { pluginId: PLUGIN, providerId: "csv", assetIds: [1], destPath: "D:\out\export.csv" }))
    expect(pick).toHaveBeenCalledWith("export.csv", [{ name: "CSV 내보내기", extensions: ["csv"] }])
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("CSV 내보내기 내보내기를 마쳤습니다. (2 KB)"))
  })


  it("T-ui-pmenu-22 [Boundary] cancelling the save dialog does nothing", async () => {
    // Arrange
    contributions = { menuItems: [], exportProviders: [provider] }
    const { user, api } = await openMenu([make(1)])
    vi.spyOn(api, "pickSaveFile").mockResolvedValue(null)
    // Act
    await user.click(screen.getByRole("menuitem", { name: "CSV 내보내기…" }))
    // Assert
    await waitFor(() => expect(api.pickSaveFile).toHaveBeenCalled())
    expect(invokeMock).not.toHaveBeenCalledWith("plugin_export", expect.anything())
    expect(toast.success).not.toHaveBeenCalled()
    expect(toast.error).not.toHaveBeenCalled()
  })

  it("T-ui-pmenu-23 [External] a rejected export toasts the mapped error, not the raw code", async () => {
    // Arrange
    contributions = { menuItems: [], exportProviders: [provider] }
    exportError = { code: "invalid_input" }
    const { user, api } = await openMenu([make(1)])
    vi.spyOn(api, "pickSaveFile").mockResolvedValue("D:/MAMData/x.csv")
    // Act
    await user.click(screen.getByRole("menuitem", { name: "CSV 내보내기…" }))
    // Assert
    await waitFor(() => expect(toast.error).toHaveBeenCalledTimes(1))
    const message = vi.mocked(toast.error).mock.calls[0][0]
    expect(message).not.toContain("invalid_input")
    expect(message).not.toContain("errors.")
    expect(toast.success).not.toHaveBeenCalled()
  })

  it("T-ui-pmenu-24 [Boundary] 5,000 assets is allowed, 5,001 disables the provider with an explanation", async () => {
    // Arrange
    contributions = { menuItems: [], exportProviders: [provider] }
    const many = (n: number) => Array.from({ length: n }, (_, i) => make(i + 1))
    // Act / Assert at the limit
    const { queryClient } = await openMenu(many(5000), {}, many(5000).map((a) => a.assetId))
    expect(screen.getByRole("menuitem", { name: "CSV 내보내기…" })).not.toHaveAttribute("aria-disabled", "true")
    queryClient.clear()
  })

  it("T-ui-pmenu-25 [Boundary] 5,001 selected: the item is disabled and states the limit", async () => {
    // Arrange
    contributions = { menuItems: [], exportProviders: [provider] }
    const assets = Array.from({ length: 5001 }, (_, i) => make(i + 1))
    // Act
    await openMenu(assets.slice(0, 2), {}, assets.map((a) => a.assetId))
    // Assert
    const el = screen.getByRole("menuitem", { name: /CSV 내보내기…/ })
    expect(el).toHaveAttribute("aria-disabled", "true")
    expect(el).toHaveTextContent("최대 5,000개까지")
  })
})
