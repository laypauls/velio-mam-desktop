// Categories: Happy (T-ui-subtitle-sel-01, T-ui-subtitle-ctx-01/03) / Business rule (T-ui-subtitle-sel-02/03, T-ui-subtitle-ctx-02) / Boundary (T-ui-subtitle-sel-04 unloaded items)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { TooltipProvider } from "@/components/ui/tooltip"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary } from "@/lib/ipc-contract"
import type { AiSettings } from "@/lib/ipc-contract-ai"
import { resetAllStores, useSelectionStore } from "@/stores"
import { AssetContextMenu } from "./AssetContextMenu"
import { SelectionBar } from "./SelectionBar"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn(), info: vi.fn(), warning: vi.fn() } }))

const make = (assetId: number, mediaType: AssetSummary["mediaType"]): AssetSummary => ({
  assetId,
  fileName: `f${assetId}.${mediaType === "video" ? "mp4" : mediaType === "audio" ? "wav" : "jpg"}`,
  mediaType,
  availability: "online",
  width: 100,
  height: 100,
  durationMs: mediaType === "image" ? null : 600_000,
  fileSize: 1,
  capturedAt: null,
  modifiedAt: 0,
  favorite: false,
  rating: 0,
  thumbState: "pending",
  aiState: "none",
  title: null,
  revision: 1,
})

class NoopResizeObserver {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

beforeEach(() => {
  resetAllStores()
  vi.stubGlobal("ResizeObserver", NoopResizeObserver) // Radix Tooltip(Popper) 가 필요로 한다
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.clearAllMocks()
  vi.unstubAllGlobals()
})

function setup(
  assets: AssetSummary[],
  ui: (lookup: (id: number) => AssetSummary | undefined) => ReactNode,
  localStt: AiSettings["localStt"] = { enabled: true, model: "base" }
) {
  const api = createMockApi({ count: 10, ai: { consent: true } })
  const base = api.getAiSettings.bind(api)
  vi.spyOn(api, "getAiSettings").mockImplementation(async () => ({ ...(await base()), localStt }))
  const analyze = vi.spyOn(api, "analyzeAssets").mockResolvedValue({ queued: 2, skipped: 0 })
  const byId = new Map(assets.map((a) => [a.assetId, a]))
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ApiProvider api={api}>
        <TooltipProvider>{ui((id) => byId.get(id))}</TooltipProvider>
      </ApiProvider>
    </QueryClientProvider>
  )
  return { analyze, user: userEvent.setup() }
}

const bar = (lookup: (id: number) => AssetSummary | undefined) => <SelectionBar lookup={lookup} />
const menuOf = (assets: AssetSummary[]) => (lookup: (id: number) => AssetSummary | undefined) => (
  <AssetContextMenu lookup={lookup}>
    {assets.map((a) => (
      <div key={a.assetId} data-asset-id={a.assetId}>
        {a.fileName}
      </div>
    ))}
  </AssetContextMenu>
)
const openMenuOn = (name: string) => fireEvent.contextMenu(screen.getByText(name))
const subtitleButton = async () =>
  within(await screen.findByRole("toolbar", { name: "선택 항목 일괄 작업" })).getByRole("button", { name: "자막 생성" })

describe("selection bar: 자막 생성", () => {
  it("T-ui-subtitle-sel-01 [Happy] videos selected → one click requests stt for the whole selection immediately (no dialog)", async () => {
    // Arrange
    useSelectionStore.getState().selectAll([1, 2])
    const { analyze, user } = setup([make(1, "video"), make(2, "audio")], bar)
    const button = await subtitleButton()
    await waitFor(() => expect(button).toBeEnabled())
    // Act
    await user.click(button)
    // Assert
    await waitFor(() => expect(analyze).toHaveBeenCalledWith([1, 2], ["stt"], false))
    expect(screen.queryByRole("dialog")).toBeNull()
    expect(screen.queryByRole("alertdialog")).toBeNull()
  })

  it("T-ui-subtitle-sel-02 [Business rule] all images → disabled, tooltip gives the reason, no IPC", async () => {
    // Arrange
    useSelectionStore.getState().selectAll([1, 2])
    const { analyze } = setup([make(1, "image"), make(2, "image")], bar)
    const button = await subtitleButton()
    // Act
    act(() => button.parentElement?.focus())
    // Assert
    expect(button).toBeDisabled()
    expect(await screen.findByRole("tooltip")).toHaveTextContent("이미지에는 자막을 만들 수 없습니다.")
    fireEvent.click(button)
    expect(analyze).not.toHaveBeenCalled()
  })

  it("T-ui-subtitle-sel-03 [Business rule] subtitles switched off → disabled with the 'turn it on' reason", async () => {
    // Arrange
    useSelectionStore.getState().selectAll([1, 2])
    const { analyze } = setup([make(1, "video"), make(2, "video")], bar, { enabled: false, model: "base" })
    // 버튼은 설정이 도착하면 툴팁 래퍼 안으로 다시 마운트되므로 비활성이 될 때까지 다시 조회한다
    await waitFor(async () => expect(await subtitleButton()).toBeDisabled())
    const button = await subtitleButton()
    // Act
    act(() => button.parentElement?.focus())
    // Assert
    expect(await screen.findByRole("tooltip")).toHaveTextContent("자막이 꺼져 있습니다")
    expect(analyze).not.toHaveBeenCalled()
  })

  it("T-ui-subtitle-sel-04 [Boundary] selection with not-yet-loaded items counts as playable → enabled", async () => {
    // Arrange: id 2 is not in the loaded blocks
    useSelectionStore.getState().selectAll([1, 2])
    setup([make(1, "image")], bar)
    // Act
    const button = await subtitleButton()
    // Assert
    expect(button).toBeEnabled()
  })
})

describe("context menu: 자막 생성", () => {
  it("T-ui-subtitle-ctx-01 [Happy] single video → item requests that id only with force=false", async () => {
    // Arrange
    const assets = [make(1, "video"), make(2, "video")]
    const { analyze, user } = setup(assets, menuOf(assets))
    // Act
    openMenuOn("f2.mp4")
    await user.click(await screen.findByRole("menuitem", { name: "자막 생성" }))
    // Assert
    await waitFor(() => expect(analyze).toHaveBeenCalledWith([2], ["stt"], false))
  })

  it("T-ui-subtitle-ctx-02 [Business rule] single image → item hidden", async () => {
    // Arrange
    const assets = [make(1, "image")]
    setup(assets, menuOf(assets))
    // Act
    openMenuOn("f1.jpg")
    await screen.findByRole("menuitem", { name: "태그…" })
    // Assert
    expect(screen.queryByRole("menuitem", { name: /자막 생성/ })).toBeNull()
  })

  it("T-ui-subtitle-ctx-03 [Happy] multi selection → label carries the count and the whole selection is requested", async () => {
    // Arrange
    const assets = [make(1, "video"), make(2, "image"), make(3, "audio")]
    useSelectionStore.getState().selectAll([1, 2, 3])
    const { analyze, user } = setup(assets, menuOf(assets))
    // Act
    openMenuOn("f1.mp4")
    await user.click(await screen.findByRole("menuitem", { name: "자막 생성 (3개)" }))
    // Assert
    await waitFor(() => expect(analyze).toHaveBeenCalledWith([1, 2, 3], ["stt"], false))
  })
})
