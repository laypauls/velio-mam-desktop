// Categories: Happy (T-ui-ctxmenu-view-01, 02) / Business rule (T-ui-ctxmenu-view-03) / Boundary (T-ui-ctxmenu-view-04)
// Regression guard with the global native-menu guard installed: gallery menu (T-ui-ctxmenu-view-05), integrated track menu (T-ui-ctxmenu-view-06)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { AssetContextMenu } from "@/components/gallery/AssetContextMenu"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { installNativeContextMenuGuard } from "@/lib/nativeContextMenu"
import { resetAllStores, useActionsStore, useViewStore } from "@/stores"
import { mountHtmlPlayer } from "@/test/ranges"
import { ContentViewer } from "./ContentViewer"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), warning: vi.fn(), info: vi.fn(), success: vi.fn() } }))

class NoopResizeObserver {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

let uninstall: (() => void) | null = null
beforeEach(() => {
  resetAllStores()
  vi.stubGlobal("ResizeObserver", NoopResizeObserver)
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  uninstall = installNativeContextMenuGuard({ isDev: false })
})
afterEach(() => {
  uninstall?.()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function openImageViewer() {
  const api = createMockApi({ count: 20 })
  const base = await api.getAssetDetail(3)
  vi.spyOn(api, "getAssetDetail").mockImplementation((id) => Promise.resolve({ ...base, assetId: id, mediaType: "image", playable: true, availability: "online", favorite: false, fileName: "pic.jpg", title: null }))
  vi.spyOn(api, "mediaUrl").mockImplementation((id) => `media://${id}`)
  const setFavorite = vi.spyOn(api, "setFavorite")
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <ContentViewer />
      </QueryClientProvider>
    </ApiProvider>
  )
  act(() => useViewStore.getState().openPreview(3))
  await screen.findByRole("img", { name: "pic.jpg" })
  return { setFavorite }
}

describe("content viewer context menu", () => {
  it("T-ui-ctxmenu-view-01 [Happy] right-click on the image stage opens the asset menu without the preview item", async () => {
    // Arrange
    await openImageViewer()
    // Act
    fireEvent.contextMenu(screen.getByTestId("image-stage"))
    // Assert
    const menu = await screen.findByRole("menu")
    expect(within(menu).getByRole("menuitem", { name: "탐색기에서 보기" })).toBeInTheDocument()
    expect(within(menu).getByRole("menuitem", { name: "즐겨찾기에 추가" })).toBeInTheDocument()
    expect(within(menu).getByRole("menuitem", { name: "태그…" })).toBeInTheDocument()
    expect(within(menu).queryByRole("menuitem", { name: "미리보기 열기" })).not.toBeInTheDocument()
    expect(within(menu).queryByText(/미리보기/)).not.toBeInTheDocument()
  })

  it("T-ui-ctxmenu-view-02 [Happy] favorite from the viewer menu targets only the viewed asset id", async () => {
    // Arrange
    const user = userEvent.setup()
    const { setFavorite } = await openImageViewer()
    fireEvent.contextMenu(screen.getByTestId("image-stage"))
    // Act
    await user.click(await screen.findByRole("menuitem", { name: "즐겨찾기에 추가" }))
    // Assert
    expect(setFavorite).toHaveBeenCalledWith([3], true)
  })

  it("T-ui-ctxmenu-view-03 [Business rule] the tags item opens the bulk tag state for [assetId]", async () => {
    // Arrange
    const user = userEvent.setup()
    await openImageViewer()
    fireEvent.contextMenu(screen.getByTestId("image-stage"))
    // Act
    await user.click(await screen.findByRole("menuitem", { name: "태그…" }))
    // Assert
    expect(useActionsStore.getState().bulkTagIds).toEqual([3])
  })

  it("T-ui-ctxmenu-view-04 [Boundary] right-click on the video stage opens the menu, but the control bar (outside the stage) does not", async () => {
    // Arrange
    const { el } = await mountHtmlPlayer({ ranges: [] })
    // Act 1: 컨트롤 바(재생 버튼) 위 — 앱 메뉴 없음, 기본 메뉴는 전역 가드가 막는다
    const play = screen.getAllByRole("button", { name: /재생|일시정지/ })[0]
    const controlsAllowed = fireEvent.contextMenu(play)
    // Assert 1
    expect(controlsAllowed).toBe(false)
    expect(screen.queryByRole("menu")).not.toBeInTheDocument()
    // Act 2: 영상 위
    fireEvent.contextMenu(el)
    // Assert 2
    const menu = await screen.findByRole("menu")
    expect(within(menu).getByRole("menuitem", { name: "탐색기에서 보기" })).toBeInTheDocument()
  })
})

describe("regression guard — existing menus with the global guard installed", () => {
  it("T-ui-ctxmenu-view-05 [Business rule] gallery AssetContextMenu still opens", async () => {
    // Arrange
    const api = createMockApi({ count: 10 })
    const summary = await api.getAssetDetail(1)
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={queryClient}>
        <ApiProvider api={api}>
          <AssetContextMenu lookup={(id) => (id === 1 ? summary : undefined)}>
            <div data-asset-id={1}>card-1</div>
            <div>blank</div>
          </AssetContextMenu>
        </ApiProvider>
      </QueryClientProvider>
    )
    // Act
    fireEvent.contextMenu(screen.getByText("card-1"))
    // Assert
    const menu = await screen.findByRole("menu")
    expect(within(menu).getByRole("menuitem", { name: "미리보기 열기" })).toBeInTheDocument()
  })

  it("T-ui-ctxmenu-view-06 [Business rule] integrated track menu still opens", async () => {
    // Arrange
    const { lane } = await mountHtmlPlayer({ ranges: [{ startMs: 10_000, endMs: 20_000 }] })
    // Act: 구간 박스 위 → 구간 항목, 빈 곳 → 모든 구간 삭제만
    fireEvent.contextMenu(within(lane).getByRole("button", { name: /구간 1/ }))
    // Assert
    const menu = await screen.findByRole("menu")
    expect(within(menu).getByRole("menuitem", { name: /구간 시작으로 이동/ })).toBeInTheDocument()
  })
})
