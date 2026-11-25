// Categories: Happy (T-ui-scene-09c single video, 09d multi selection) / Business rule (T-ui-scene-09e image-only hides the item) / External (T-ui-scene-09f request failure → toast)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary } from "@/lib/ipc-contract"
import { resetAllStores, useSelectionStore } from "@/stores"
import { AssetContextMenu } from "./AssetContextMenu"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn(), info: vi.fn(), warning: vi.fn() } }))

const make = (assetId: number, mediaType: AssetSummary["mediaType"]): AssetSummary => ({
  assetId,
  fileName: `f${assetId}.${mediaType === "video" ? "mp4" : "jpg"}`,
  mediaType,
  availability: "online",
  width: 100,
  height: 100,
  durationMs: mediaType === "video" ? 5000 : null,
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

function setup(assets: AssetSummary[]) {
  const api = createMockApi({ count: 10 })
  const request = vi.spyOn(api, "sceneDetectRequest").mockResolvedValue({ queued: 1, skipped: 0 })
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
  return { request, user: userEvent.setup() }
}
const openMenuOn = (name: string) => fireEvent.contextMenu(screen.getByText(name))

describe("context menu scene item", () => {
  it("T-ui-scene-09c [Happy] a single video offers 장면전환점 추출, which requests only that asset with default options and toasts the queued count", async () => {
    // Arrange
    const { request, user } = setup([make(1, "video"), make(2, "video")])
    // Act
    openMenuOn("f2.mp4")
    await user.click(await screen.findByRole("menuitem", { name: "장면전환점 추출" }))
    // Assert
    await waitFor(() => expect(request).toHaveBeenCalledWith([2], {}))
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("동영상 1개의 장면전환점 추출을 대기열에 등록했습니다."))
  })

  it("T-ui-scene-09d [Happy] inside a multi selection the request carries every selected id (the server skips non-videos)", async () => {
    // Arrange
    const { request, user } = setup([make(1, "video"), make(2, "image"), make(3, "video")])
    useSelectionStore.getState().selectAll([1, 2, 3])
    // Act
    openMenuOn("f1.mp4")
    await user.click(await screen.findByRole("menuitem", { name: "장면전환점 추출" }))
    // Assert
    await waitFor(() => expect(request).toHaveBeenCalledWith([1, 2, 3], {}))
  })

  it("T-ui-scene-09e [Business rule] a single image has no 장면전환점 추출 item", async () => {
    setup([make(1, "image")])
    openMenuOn("f1.jpg")
    await screen.findByRole("menuitem", { name: "태그…" })
    expect(screen.queryByRole("menuitem", { name: "장면전환점 추출" })).toBeNull()
  })

  it("T-ui-scene-09f [External] a rejected request shows the error toast and does not report success", async () => {
    // Arrange
    const { request, user } = setup([make(1, "video")])
    request.mockRejectedValueOnce({ code: "internal", message: "boom" })
    // Act
    openMenuOn("f1.mp4")
    await user.click(await screen.findByRole("menuitem", { name: "장면전환점 추출" }))
    // Assert
    await waitFor(() => expect(toast.error).toHaveBeenCalledTimes(1))
    expect(toast.success).not.toHaveBeenCalled()
  })
})

