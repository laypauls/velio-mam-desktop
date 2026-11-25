// Categories: Happy (T-ui-ctxmenu-01, 02, 03, 05, 07) / Business rule (T-ui-ctxmenu-04, 06) / External failure (T-ui-ctxmenu-08)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary } from "@/lib/ipc-contract"
import { resetAllStores, useActionsStore, useAiStore, useSelectionStore } from "@/stores"
import { AssetContextMenu } from "./AssetContextMenu"

const make = (assetId: number, aiState: AssetSummary["aiState"], availability: AssetSummary["availability"] = "online"): AssetSummary => ({
  assetId,
  fileName: `f${assetId}.jpg`,
  mediaType: "image",
  availability,
  width: 100,
  height: 100,
  durationMs: null,
  fileSize: 1,
  capturedAt: null,
  modifiedAt: 0,
  favorite: false,
  rating: 0,
  thumbState: "pending",
  aiState,
  title: null,
  revision: 1,
})

beforeEach(() => resetAllStores())
afterEach(() => vi.restoreAllMocks())

function setup(assets: AssetSummary[]) {
  const api = createMockApi({ count: 10 })
  const analyzeSpy = vi.spyOn(api, "analyzeAssets")
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
  return { analyzeSpy, user: userEvent.setup() }
}

const openMenuOn = (name: string) => fireEvent.contextMenu(screen.getByText(name))

describe("context menu tag / AI items", () => {
  it("T-ui-ctxmenu-01 [Happy] tag item on a single target opens the bulk tag state for that id", async () => {
    // Arrange
    const { user } = setup([make(1, "none"), make(2, "none")])
    // Act
    openMenuOn("f2.jpg")
    await user.click(await screen.findByRole("menuitem", { name: "태그…" }))
    // Assert
    expect(useActionsStore.getState().bulkTagIds).toEqual([2])
  })

  it("T-ui-ctxmenu-02 [Happy] tag item inside a multi selection opens the bulk tag state for all selected ids", async () => {
    // Arrange
    const { user } = setup([make(1, "none"), make(2, "none"), make(3, "none")])
    useSelectionStore.getState().selectAll([1, 3])
    // Act
    openMenuOn("f3.jpg")
    await user.click(await screen.findByRole("menuitem", { name: "태그…" }))
    // Assert
    expect([...(useActionsStore.getState().bulkTagIds ?? [])].sort()).toEqual([1, 3])
  })

  it.each(["none", "failed"] as const)("T-ui-ctxmenu-03 [Happy] aiState %s: 'AI 분석' calls analyzeAssets([id], null, false)", async (aiState) => {
    // Arrange
    const { user, analyzeSpy } = setup([make(5, aiState)])
    // Act
    openMenuOn("f5.jpg")
    await user.click(await screen.findByRole("menuitem", { name: "AI 분석" }))
    // Assert
    await waitFor(() => expect(analyzeSpy).toHaveBeenCalledWith([5], null, false))
    expect(screen.queryByRole("menuitem", { name: "AI 재분석" })).not.toBeInTheDocument()
  })

  it("T-ui-ctxmenu-04 [Business rule] aiState done: shows 재분석, asks for confirmation, confirm calls force=true", async () => {
    // Arrange
    const { user, analyzeSpy } = setup([make(6, "done")])
    // Act 1
    openMenuOn("f6.jpg")
    expect(screen.queryByRole("menuitem", { name: "AI 분석" })).not.toBeInTheDocument()
    await user.click(await screen.findByRole("menuitem", { name: "AI 재분석" }))
    // Assert 1: 확인 전에는 호출하지 않는다 (메뉴가 닫혀도 Dialog 는 남는다)
    const dialog = await screen.findByRole("alertdialog", { name: "재분석할까요?" })
    expect(analyzeSpy).not.toHaveBeenCalled()
    // Act 2
    await user.click(await screen.findByRole("button", { name: "재분석" }))
    // Assert 2
    await waitFor(() => expect(analyzeSpy).toHaveBeenCalledWith([6], null, true))
    expect(dialog).not.toBeVisible()
  })

  it("T-ui-ctxmenu-05 [Business rule] cancelling the reanalyze confirmation does not call analyzeAssets", async () => {
    // Arrange
    const { user, analyzeSpy } = setup([make(6, "done")])
    openMenuOn("f6.jpg")
    await user.click(await screen.findByRole("menuitem", { name: "AI 재분석" }))
    await screen.findByRole("alertdialog", { name: "재분석할까요?" })
    // Act
    await user.click(screen.getByRole("button", { name: "취소" }))
    // Assert
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument())
    expect(analyzeSpy).not.toHaveBeenCalled()
  })

  it("T-ui-ctxmenu-06 [Business rule] multi selection: 'AI 분석…' opens the bulk analyze state regardless of aiState, no direct call", async () => {
    // Arrange
    const { user, analyzeSpy } = setup([make(1, "done"), make(2, "none")])
    useSelectionStore.getState().selectAll([1, 2])
    // Act
    openMenuOn("f1.jpg")
    expect(screen.queryByRole("menuitem", { name: "AI 재분석" })).not.toBeInTheDocument()
    await user.click(await screen.findByRole("menuitem", { name: "AI 분석…" }))
    // Assert
    expect([...(useAiStore.getState().bulkAnalyzeIds ?? [])].sort()).toEqual([1, 2])
    expect(analyzeSpy).not.toHaveBeenCalled()
  })

  it("T-ui-ctxmenu-07 [Boundary] offline asset still offers tag and AI items enabled", async () => {
    // Arrange
    setup([make(9, "none", "offline")])
    // Act
    openMenuOn("f9.jpg")
    // Assert
    expect(await screen.findByRole("menuitem", { name: "AI 분석" })).not.toHaveAttribute("data-disabled")
    expect(screen.getByRole("menuitem", { name: "태그…" })).not.toHaveAttribute("data-disabled")
  })

  it("T-ui-ctxmenu-08 [External failure] analyzeAssets rejecting shows an error toast and does not crash", async () => {
    // Arrange
    const { user, analyzeSpy } = setup([make(5, "none")])
    analyzeSpy.mockRejectedValue({ code: "internal", message: "boom" })
    const errorToast = vi.spyOn(toast, "error").mockReturnValue("t")
    // Act
    openMenuOn("f5.jpg")
    await user.click(await screen.findByRole("menuitem", { name: "AI 분석" }))
    // Assert
    await waitFor(() => expect(errorToast).toHaveBeenCalledWith("알 수 없는 오류가 발생했습니다."))
  })
})
