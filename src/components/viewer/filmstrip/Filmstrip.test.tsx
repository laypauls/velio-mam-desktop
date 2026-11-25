// Categories: Happy (T-ui-filmstrip-01 enter/return, 03 keyboard, 04 aria-selected, 10 hide toggle + image tools) / Boundary (T-ui-filmstrip-02 empty, 05 page boundary, 06 End across pages)
//             Business rule (T-ui-filmstrip-07 offline tile, 08 duration badge, 09 multi-select, 11 close paths, 12 height handle, 13 fullscreen, 15 top bar toggle, 17 leave) / Concurrency (T-ui-filmstrip-14 no remount, 16 new search)
// 필름스트립 보기 모드 — 앱 전체를 렌더하고 스토어·접근성 역할로 검증한다 (ResizeObserver/rAF 는 stubLayout 이 결정적으로 스텁)
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import type { UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary, GalleryPaging } from "@/lib/ipc-contract"
import { useSearchStore, useSelectionStore, useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { stubMedia, useStubbedDetail } from "@/test/media"
import { fakeAssets, mockFixedResults } from "@/test/searchMock"

const win = vi.hoisted(() => ({ setFullscreen: vi.fn(), isMaximized: vi.fn(), maximize: vi.fn(), setTheme: vi.fn() }))
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => win }))

beforeEach(() => {
  win.setFullscreen.mockReset().mockResolvedValue(undefined)
  win.isMaximized.mockReset().mockResolvedValue(false)
  win.maximize.mockReset().mockResolvedValue(undefined)
  win.setTheme.mockReset().mockResolvedValue(undefined)
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup(items: AssetSummary[], opts: { paging?: GalleryPaging; results?: (api: UiApi) => void } = {}) {
  stubLayout({ width: 1200, height: 800 })
  stubMedia()
  const api: UiApi = createMockApi({ count: 10 })
  await useStubbedDetail(api, { mediaType: "image" })
  // 기본 숨기기 단축키가 Esc 라 앱 창을 숨기며 Esc 를 가로챈다 (useHideShortcut) — Esc 로 보기를 닫는 경로는 이 단축키를 끈 상태로 검증한다
  await api.updateSettings({ galleryPaging: opts.paging ?? "infinite", galleryPageSize: 100, hideShortcut: null })
  mockFixedResults(api, items)
  opts.results?.(api)
  const user = userEvent.setup()
  renderApp(api)
  if (items.length > 0) await screen.findAllByRole("option")
  else await screen.findByText("아직 미디어가 없습니다")
  return { api, user }
}

const enter = () => act(() => useViewStore.getState().setViewMode("filmstrip"))
const strip = () => screen.findByRole("listbox", { name: "검색 결과" })
const tile = (name: string) => within(screen.getByRole("listbox", { name: "검색 결과" })).getByRole("option", { name: new RegExp(`^${name}`) })
const previewId = () => useViewStore.getState().previewId

describe("filmstrip view mode", () => {
  it("T-ui-filmstrip-01 [Happy] entering shows the first result (no active item) in a strip, and Esc returns to the previous list mode closing the viewer", async () => {
    // Arrange
    await setup(fakeAssets(10))
    act(() => useViewStore.getState().setViewMode("list"))
    // Act
    enter()
    // Assert: 첫 결과가 현재 항목, 분할 패널(complementary)은 없다
    await strip()
    await waitFor(() => expect(previewId()).toBe(1))
    expect(screen.queryByRole("complementary", { name: "콘텐츠 보기" })).not.toBeInTheDocument()
    expect(tile("file_1.jpg")).toHaveAttribute("aria-selected", "true")
    // Act: Esc
    fireEvent.keyDown(document.body, { key: "Escape" })
    // Assert: 직전 모드(목록)로 복귀, 보기 닫힘
    await waitFor(() => expect(useViewStore.getState().viewMode).toBe("list"))
    expect(previewId()).toBeNull()
    expect(screen.queryByRole("listbox", { name: "검색 결과" })).not.toBeInTheDocument()
  })

  it("T-ui-filmstrip-01b [Happy] entering with an active item shows that item", async () => {
    await setup(fakeAssets(10))
    act(() => useSelectionStore.getState().selectOnly(5))
    enter()
    await strip()
    await waitFor(() => expect(previewId()).toBe(5))
    expect(tile("file_5.jpg")).toHaveAttribute("aria-selected", "true")
  })

  it("T-ui-filmstrip-02 [Boundary] zero results: empty state, no strip, nothing opened", async () => {
    // Arrange
    await setup([])
    // Act
    enter()
    // Assert
    expect((await screen.findAllByText("아직 미디어가 없습니다")).length).toBeGreaterThan(0)
    expect(screen.queryByRole("listbox", { name: "검색 결과" })).not.toBeInTheDocument()
    expect(previewId()).toBeNull()
    // 격자로 돌아온다
    act(() => useViewStore.getState().setViewMode("grid"))
    expect(useViewStore.getState().viewMode).toBe("grid")
  })

  it("T-ui-filmstrip-03 [Happy] ArrowRight/ArrowLeft/Home/End move the current item; stepping past an end does nothing", async () => {
    // Arrange
    await setup(fakeAssets(10))
    act(() => useSelectionStore.getState().selectOnly(3))
    enter()
    const list = await strip()
    await waitFor(() => expect(previewId()).toBe(3))
    list.focus()
    // Act + Assert
    fireEvent.keyDown(list, { key: "ArrowRight" })
    await waitFor(() => expect(previewId()).toBe(4))
    expect(useSelectionStore.getState().activeId).toBe(4)
    fireEvent.keyDown(list, { key: "ArrowLeft" })
    await waitFor(() => expect(previewId()).toBe(3))
    fireEvent.keyDown(list, { key: "End" })
    await waitFor(() => expect(previewId()).toBe(10))
    fireEvent.keyDown(list, { key: "ArrowRight" })
    expect(previewId()).toBe(10)
    fireEvent.keyDown(list, { key: "Home" })
    await waitFor(() => expect(previewId()).toBe(1))
    fireEvent.keyDown(list, { key: "ArrowLeft" })
    expect(previewId()).toBe(1)
  })

  it("T-ui-filmstrip-04 [Happy] exactly one tile is aria-selected (the current one); clicking another tile moves it and the detail follows the active item", async () => {
    // Arrange
    const { user } = await setup(fakeAssets(10))
    enter()
    await strip()
    await waitFor(() => expect(previewId()).toBe(1))
    // Act
    await user.click(tile("file_6.jpg"))
    // Assert
    await waitFor(() => expect(previewId()).toBe(6))
    const selected = within(screen.getByRole("listbox", { name: "검색 결과" })).getAllByRole("option", { selected: true })
    expect(selected).toHaveLength(1)
    expect(selected[0]).toHaveAttribute("id", "filmstrip-opt-6")
    expect(selected[0]).toHaveAttribute("aria-posinset", "6")
    expect(selected[0]).toHaveAttribute("aria-setsize", "10")
    expect(useSelectionStore.getState().activeId).toBe(6)
  })

  it("T-ui-filmstrip-05 [Boundary] paged mode: ArrowRight on a page's last item crosses to the next page and moves galleryPage; ArrowLeft returns", async () => {
    // Arrange
    await setup(fakeAssets(1000), { paging: "paged" })
    act(() => useSelectionStore.getState().selectOnly(100))
    enter()
    const list = await strip()
    await waitFor(() => expect(previewId()).toBe(100))
    list.focus()
    // Act
    fireEvent.keyDown(list, { key: "ArrowRight" })
    // Assert
    await waitFor(() => expect(previewId()).toBe(101))
    expect(useViewStore.getState().galleryPage).toBe(1)
    // Act: 되돌아가기
    await waitFor(() => expect(tile("file_101.jpg")).toBeInTheDocument())
    fireEvent.keyDown(screen.getByRole("listbox", { name: "검색 결과" }), { key: "ArrowLeft" })
    await waitFor(() => expect(previewId()).toBe(100))
    expect(useViewStore.getState().galleryPage).toBe(0)
  })

  it("T-ui-filmstrip-06 [Boundary] paged mode: End jumps to the last page's last item", async () => {
    await setup(fakeAssets(1000), { paging: "paged" })
    enter()
    const list = await strip()
    await waitFor(() => expect(previewId()).toBe(1))
    list.focus()
    fireEvent.keyDown(list, { key: "End" })
    await waitFor(() => expect(previewId()).toBe(1000))
    expect(useViewStore.getState().galleryPage).toBe(9)
  })

  it("T-ui-filmstrip-07 [Business rule] an offline item gets a hatched tile and an offline label (not removed from the strip)", async () => {
    // Arrange
    const items = fakeAssets(5).map((a) => (a.assetId === 3 ? { ...a, availability: "offline" as const } : a))
    await setup(items)
    // Act
    enter()
    await strip()
    // Assert
    const off = tile("file_3.jpg")
    expect(off).toHaveAttribute("aria-label", expect.stringContaining("오프라인"))
    expect(within(off).getByTestId("filmstrip-offline")).toBeInTheDocument()
    expect(within(tile("file_2.jpg")).queryByTestId("filmstrip-offline")).not.toBeInTheDocument()
  })

  it("T-ui-filmstrip-08 [Business rule] video and audio tiles show the duration; images do not; favorite and rating chips appear", async () => {
    // Arrange
    const items = fakeAssets(4).map((a) =>
      a.assetId === 2
        ? { ...a, mediaType: "video" as const, fileName: "clip.mp4", durationMs: 90_000, favorite: true, rating: 4 }
        : a.assetId === 3
          ? { ...a, mediaType: "audio" as const, fileName: "song.wav", durationMs: 205_000 }
          : a
    )
    await setup(items)
    // Act
    enter()
    await strip()
    // Assert
    expect(within(tile("clip.mp4")).getByText("1:30")).toBeInTheDocument()
    expect(within(tile("clip.mp4")).getByText("4")).toBeInTheDocument()
    expect(within(tile("song.wav")).getByText("3:25")).toBeInTheDocument()
    expect(within(tile("file_1.jpg")).queryByText(/\d:\d\d/)).not.toBeInTheDocument()
  })

  it("T-ui-filmstrip-09 [Business rule] Ctrl+click toggles multi-selection (existing rules): the tile gets the multi-select mark and the current item follows", async () => {
    // Arrange
    await setup(fakeAssets(10))
    enter()
    await strip()
    await waitFor(() => expect(previewId()).toBe(1))
    // Act
    fireEvent.click(tile("file_4.jpg"), { ctrlKey: true })
    // Assert
    expect([...useSelectionStore.getState().ids].sort((a, b) => a - b)).toEqual([1, 4])
    await waitFor(() => expect(previewId()).toBe(4))
    expect(tile("file_1.jpg")).toHaveAttribute("data-multi-selected", "true")
    expect(tile("file_4.jpg")).toHaveAttribute("aria-selected", "true")
    // Act: Shift+click = 범위
    fireEvent.click(tile("file_7.jpg"), { shiftKey: true })
    expect([...useSelectionStore.getState().ids].sort((a, b) => a - b)).toEqual([4, 5, 6, 7])
  })

  it("T-ui-filmstrip-10 [Happy] the strip toggle hides the strip (stage stays) and a restore bar shows it again; the image tools live in the toolbar slot", async () => {
    // Arrange
    const { user } = await setup(fakeAssets(10))
    enter()
    await strip()
    const toolbar = screen.getByRole("toolbar", { name: "보기 도구" })
    // 이미지 도구는 ImageViewer 의 것을 툴바 슬롯으로 포털한 것 (복제 아님)
    expect(await within(toolbar).findByRole("toolbar", { name: "이미지 보기 도구" })).toBeInTheDocument()
    expect(within(toolbar).getByRole("button", { name: "확대" })).toBeInTheDocument()
    // Act
    await user.click(within(toolbar).getByRole("button", { name: "필름스트립 숨기기" }))
    // Assert
    expect(screen.queryByRole("listbox", { name: "검색 결과" })).not.toBeInTheDocument()
    expect(screen.getByTestId("image-stage")).toBeInTheDocument()
    // Act
    expect(screen.getAllByRole("button", { name: "필름스트립 표시" })).toHaveLength(2) // 툴바 토글 + 하단 복원 줄
    await user.click(screen.getAllByRole("button", { name: "필름스트립 표시" })[1])
    expect(await strip()).toBeInTheDocument()
  })

  it("T-ui-filmstrip-11 [Business rule] close paths: the ✕ button and Q both return to the previous mode and close the viewer", async () => {
    // Arrange
    const { user } = await setup(fakeAssets(10))
    enter()
    await strip()
    // Act: ✕
    await user.click(within(screen.getByRole("toolbar", { name: "보기 도구" })).getByRole("button", { name: /^보기 영역 닫기/ }))
    // Assert
    expect(useViewStore.getState().viewMode).toBe("grid")
    expect(previewId()).toBeNull()
    // Act: Q
    enter()
    await strip()
    await waitFor(() => expect(previewId()).toBe(1))
    await user.keyboard("q")
    // Assert
    await waitFor(() => expect(useViewStore.getState().viewMode).toBe("grid"))
    expect(previewId()).toBeNull()
  })

  it("T-ui-filmstrip-12 [Boundary] the height handle resizes with the keyboard in 8px steps within 72..160", async () => {
    // Arrange
    const { user } = await setup(fakeAssets(10))
    enter()
    await strip()
    const handle = screen.getByRole("separator", { name: "필름스트립 높이 조절" })
    expect(handle).toHaveAttribute("aria-valuenow", "104")
    handle.focus()
    // Act
    await user.keyboard("{ArrowUp}")
    // Assert
    expect(useViewStore.getState().filmstripHeight).toBe(112)
    expect(useViewStore.getState().layoutRev).toBeGreaterThan(0)
    expect(handle).toHaveAttribute("aria-valuenow", "112")
    for (let i = 0; i < 20; i++) await user.keyboard("{ArrowUp}")
    expect(useViewStore.getState().filmstripHeight).toBe(160)
    for (let i = 0; i < 20; i++) await user.keyboard("{ArrowDown}")
    expect(useViewStore.getState().filmstripHeight).toBe(72)
  })

  it("T-ui-filmstrip-13 [Business rule] fullscreen hides the toolbar and the strip but keeps the stage mounted (no remount); leaving restores them", async () => {
    // Arrange
    await setup(fakeAssets(10))
    enter()
    await strip()
    const stage = await screen.findByTestId("image-stage")
    // Act
    act(() => useViewStore.getState().setFullscreen(true))
    // Assert
    expect(screen.queryByRole("toolbar", { name: "보기 도구" })).not.toBeInTheDocument()
    expect(screen.queryByRole("listbox", { name: "검색 결과" })).not.toBeInTheDocument()
    expect(screen.getByTestId("image-stage")).toBe(stage)
    // Act
    act(() => useViewStore.getState().setFullscreen(false))
    expect(screen.getByRole("toolbar", { name: "보기 도구" })).toBeInTheDocument()
    expect(await strip()).toBeInTheDocument()
  })

  it("T-ui-filmstrip-14 [Concurrency] navigating inside the filmstrip keeps the toolbar and the strip mounted (same DOM nodes)", async () => {
    // Arrange
    await setup(fakeAssets(10))
    enter()
    const list = await strip()
    const toolbar = screen.getByRole("toolbar", { name: "보기 도구" })
    await waitFor(() => expect(previewId()).toBe(1))
    list.focus()
    // Act
    fireEvent.keyDown(list, { key: "ArrowRight" })
    await waitFor(() => expect(previewId()).toBe(2))
    // Assert
    expect(screen.getByRole("toolbar", { name: "보기 도구" })).toBe(toolbar)
    expect(screen.getByRole("listbox", { name: "검색 결과" })).toBe(list)
  })

  it("T-ui-filmstrip-15 [Business rule] top bar 3-way toggle: choosing 필름스트립 enters the mode and disables the card display menu; gallery returns and closes the viewer", async () => {
    // Arrange
    const { user } = await setup(fakeAssets(10))
    const group = screen.getByRole("radiogroup", { name: "보기 방식" })
    // Act
    await user.click(within(group).getByRole("radio", { name: "필름스트립" }))
    // Assert
    expect(useViewStore.getState().viewMode).toBe("filmstrip")
    expect(screen.getByRole("button", { name: "표시 옵션" })).toBeDisabled()
    expect(within(group).getByRole("radio", { name: "필름스트립" })).toBeChecked()
    await waitFor(() => expect(previewId()).toBe(1))
    // Act
    await user.click(within(group).getByRole("radio", { name: "갤러리" }))
    // Assert
    expect(useViewStore.getState().viewMode).toBe("grid")
    expect(previewId()).toBeNull()
    expect(screen.getByRole("button", { name: "표시 옵션" })).toBeEnabled()
  })

  it("T-ui-filmstrip-16 [Concurrency] a new search while in filmstrip drops the old item and shows the new first result", async () => {
    // Arrange: 질의에 따라 다른 결과
    const all = fakeAssets(10)
    const other = fakeAssets(3, 101)
    await setup(all, { results: (api) => mockFixedResults(api, (req) => (req.query === "x" ? other : all)) })
    act(() => useSelectionStore.getState().selectOnly(5))
    enter()
    await strip()
    await waitFor(() => expect(previewId()).toBe(5))
    // Act
    act(() => useSearchStore.getState().setQuery("x"))
    // Assert
    await waitFor(() => expect(previewId()).toBe(101))
    expect(tile("file_101.jpg")).toHaveAttribute("aria-selected", "true")
  })

  it("T-ui-filmstrip-18 [Boundary] the strip's right button only browses (current item unchanged); at a paged range end it moves to the next page", async () => {
    // Arrange
    const { user } = await setup(fakeAssets(1000), { paging: "paged" })
    enter()
    await strip()
    await waitFor(() => expect(previewId()).toBe(1))
    const right = screen.getByRole("button", { name: "스트립 오른쪽으로" })
    const left = screen.getByRole("button", { name: "스트립 왼쪽으로" })
    expect(left).toBeDisabled() // 첫 쪽의 처음 — 더 갈 곳 없음
    // Act: 쪽 끝을 넘을 때까지
    for (let i = 0; i < 60 && useViewStore.getState().galleryPage === 0; i++) await user.click(right)
    // Assert
    expect(useViewStore.getState().galleryPage).toBe(1)
    expect(previewId()).toBe(1)
    await waitFor(() => expect(screen.getByRole("button", { name: "스트립 왼쪽으로" })).toBeEnabled())
  })

  it("T-ui-filmstrip-17 [Business rule] leaving filmstrip returns focus to the gallery list", async () => {
    // Arrange
    await setup(fakeAssets(10))
    enter()
    const list = await strip()
    list.focus()
    fireEvent.keyDown(list, { key: "End" })
    await waitFor(() => expect(previewId()).toBe(10))
    // Act
    fireEvent.keyDown(document.body, { key: "Escape" })
    // Assert
    await waitFor(() => expect(useViewStore.getState().viewMode).toBe("grid"))
    expect(screen.getByRole("listbox", { name: "미디어 목록" })).toHaveFocus()
  })
})

describe("filmstrip tile visuals", () => {
  it("T-ui-filmstrip-19 [Regression] the current tile has a strong primary ring (multi-selected a lighter border) and the offline hatch sits below the badges", async () => {
    // Arrange
    const items = fakeAssets(4).map((a) => (a.assetId === 2 ? { ...a, availability: "offline" as const, rating: 5, mediaType: "video" as const, fileName: "off.mp4", durationMs: 1_332_000 } : a))
    await setup(items)
    enter()
    await strip()
    await waitFor(() => expect(previewId()).toBe(1))
    fireEvent.click(tile("off.mp4"), { ctrlKey: true })
    await waitFor(() => expect(previewId()).toBe(2))
    // Assert: 현재 항목은 강한 링, 다중 선택만 된 항목은 옅은 테두리
    expect(tile("off.mp4").className).toMatch(/ring-2/)
    expect(tile("off.mp4").className).toMatch(/\bring-primary\b/)
    expect(tile("file_1.jpg").className).toMatch(/border-primary\/50/)
    expect(tile("file_1.jpg").className).not.toMatch(/ring-2/)
    // 빗금 덮개는 배지보다 먼저(아래) 그려지고, 배지는 z-10 으로 위에 놓인다
    const off = tile("off.mp4")
    const hatch = within(off).getByTestId("filmstrip-offline")
    const duration = within(off).getByText("22:12")
    const rating = within(off).getByText("5")
    expect(hatch.compareDocumentPosition(duration) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(duration.className).toMatch(/\bz-10\b/)
    expect(rating.parentElement!.className).toMatch(/\bz-10\b/)
    expect(duration.className).toMatch(/whitespace-nowrap/)
  })
})
