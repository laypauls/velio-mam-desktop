// Categories: Happy (clear + toast) / Business rule (hidden for images only, disabled at 0 records, executes immediately without a dialog) / External (failure toast)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { TooltipProvider } from "@/components/ui/tooltip"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary } from "@/lib/ipc-contract"
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

beforeEach(() => resetAllStores())
afterEach(() => {
  vi.restoreAllMocks()
  vi.clearAllMocks()
})

/** `counts`: 자산 id → 기록 여부. resumeCountAssets 는 선택 중 기록이 있는 수를 돌려준다 */
function setup(assets: AssetSummary[], withRecord: number[], ui: (lookup: (id: number) => AssetSummary | undefined) => ReactNode) {
  const api = createMockApi({ count: 10 })
  const count = vi.spyOn(api, "resumeCountAssets").mockImplementation(async (ids) => ids.filter((id) => withRecord.includes(id)).length)
  const clear = vi.spyOn(api, "resumeClearAssets").mockImplementation(async (ids) => ids.filter((id) => withRecord.includes(id)).length)
  const byId = new Map(assets.map((a) => [a.assetId, a]))
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={queryClient}>
      <ApiProvider api={api}>
        <TooltipProvider>{ui((id) => byId.get(id))}</TooltipProvider>
      </ApiProvider>
    </QueryClientProvider>
  )
  return { count, clear, user: userEvent.setup() }
}

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

describe("context menu: 이어보기 초기화", () => {
  it("T-ui-resume-60 [Happy] a video with a record: the item is enabled and clears immediately (no dialog) with a count toast", async () => {
    // Arrange
    const assets = [make(1, "video"), make(2, "video")]
    const { clear, user } = setup(assets, [2], menuOf(assets))
    // Act
    openMenuOn("f2.mp4")
    const item = await screen.findByRole("menuitem", { name: "이어보기 초기화" })
    await waitFor(() => expect(item).not.toHaveAttribute("aria-disabled", "true"))
    await user.click(item)
    // Assert
    await waitFor(() => expect(clear).toHaveBeenCalledWith([2]))
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("이어보기 기록 1건을 초기화했습니다."))
    expect(screen.queryByRole("alertdialog")).toBeNull()
  })

  it("T-ui-resume-61 [Business rule] no record among the selection → the item is disabled and does nothing", async () => {
    // Arrange
    const assets = [make(1, "audio")]
    const { clear, count } = setup(assets, [], menuOf(assets))
    // Act
    openMenuOn("f1.wav")
    const item = await screen.findByRole("menuitem", { name: "이어보기 초기화" })
    await waitFor(() => expect(count).toHaveBeenCalledWith([1]))
    // Assert
    expect(item).toHaveAttribute("aria-disabled", "true")
    fireEvent.click(item)
    expect(clear).not.toHaveBeenCalled()
  })

  it("T-ui-resume-62 [Business rule] an image-only selection hides the item and never asks for the count", async () => {
    // Arrange
    const assets = [make(1, "image")]
    const { count } = setup(assets, [], menuOf(assets))
    // Act
    openMenuOn("f1.jpg")
    await screen.findByRole("menuitem", { name: "태그…" })
    // Assert
    expect(screen.queryByRole("menuitem", { name: "이어보기 초기화" })).toBeNull()
    expect(count).not.toHaveBeenCalled()
  })

  it("T-ui-resume-63 [Happy] a mixed multi selection (image + video) shows the item and clears every selected id", async () => {
    // Arrange
    const assets = [make(1, "image"), make(2, "video"), make(3, "video")]
    useSelectionStore.getState().selectAll([1, 2, 3])
    const { clear, user } = setup(assets, [2, 3], menuOf(assets))
    // Act
    openMenuOn("f2.mp4")
    const item = await screen.findByRole("menuitem", { name: "이어보기 초기화" })
    await waitFor(() => expect(item).not.toHaveAttribute("aria-disabled", "true"))
    await user.click(item)
    // Assert
    await waitFor(() => expect(clear).toHaveBeenCalledWith([1, 2, 3]))
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("이어보기 기록 2건을 초기화했습니다."))
  })

  it("T-ui-resume-64 [External] a rejected clear shows the error toast and no success toast", async () => {
    // Arrange
    const assets = [make(1, "video")]
    const { clear, user } = setup(assets, [1], menuOf(assets))
    clear.mockRejectedValueOnce({ code: "internal", message: "boom" })
    // Act
    openMenuOn("f1.mp4")
    const item = await screen.findByRole("menuitem", { name: "이어보기 초기화" })
    await waitFor(() => expect(item).not.toHaveAttribute("aria-disabled", "true"))
    await user.click(item)
    // Assert
    await waitFor(() => expect(toast.error).toHaveBeenCalledTimes(1))
    expect(toast.success).not.toHaveBeenCalled()
  })
})

describe("selection bar: 이어보기 초기화", () => {
  it("T-ui-resume-65 [Happy] videos selected with records → enabled button clears the selection immediately and toasts the count", async () => {
    // Arrange
    const assets = [make(1, "video"), make(2, "audio")]
    useSelectionStore.getState().selectAll([1, 2])
    const { clear, user } = setup(assets, [1, 2], (lookup) => <SelectionBar lookup={lookup} />)
    const toolbar = await screen.findByRole("toolbar", { name: "선택 항목 일괄 작업" })
    const button = within(toolbar).getByRole("button", { name: "이어보기 초기화" })
    await waitFor(() => expect(button).toBeEnabled())
    // Act
    await user.click(button)
    // Assert
    await waitFor(() => expect(clear).toHaveBeenCalledWith([1, 2]))
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("이어보기 기록 2건을 초기화했습니다."))
  })

  it("T-ui-resume-66 [Business rule] disabled while no selected item has a record", async () => {
    // Arrange
    const assets = [make(1, "video"), make(2, "video")]
    useSelectionStore.getState().selectAll([1, 2])
    const { count } = setup(assets, [], (lookup) => <SelectionBar lookup={lookup} />)
    // Act
    const button = await screen.findByRole("button", { name: "이어보기 초기화" })
    await waitFor(() => expect(count).toHaveBeenCalled())
    // Assert
    expect(button).toBeDisabled()
  })

  it("T-ui-resume-67 [Business rule] images only → no 이어보기 초기화 button", async () => {
    // Arrange
    const assets = [make(1, "image"), make(2, "image")]
    useSelectionStore.getState().selectAll([1, 2])
    const { count } = setup(assets, [], (lookup) => <SelectionBar lookup={lookup} />)
    // Act
    await screen.findByRole("toolbar", { name: "선택 항목 일괄 작업" })
    // Assert
    expect(screen.queryByRole("button", { name: "이어보기 초기화" })).toBeNull()
    expect(count).not.toHaveBeenCalled()
  })
})
