// Categories: Happy (T-ui-vnav-01) / Boundary (T-ui-vnav-02 ends, 03 single result, 04 page edge) / Validation (T-ui-vnav-05 not in results, 06 unmount)
// 단독 보기의 이전/다음 콘텐츠 — 갤러리가 보기 영역에 내주는 stepper(`useViewerNavStore`)가 화살표 키와 같은 경로로 활성 항목을 옮기는지 확인한다
import { QueryClient } from "@tanstack/react-query"
import { act, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import App from "@/App"
import type { UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary, GalleryPaging } from "@/lib/ipc-contract"
import { resetAllStores, useSelectionStore, useViewStore, useViewerNavStore } from "@/stores"
import { stubLayout } from "@/test/layout"
import { fakeAssets, mockFixedResults } from "@/test/searchMock"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup(items: AssetSummary[], paging: GalleryPaging = "infinite") {
  stubLayout({ width: 1000, height: 600 })
  const api: UiApi = createMockApi({ count: 10 })
  await api.updateSettings({ galleryPaging: paging, galleryPageSize: 100 })
  mockFixedResults(api, items)
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
  resetAllStores()
  const view = render(<App api={api} queryClient={qc} />)
  await screen.findAllByRole("option")
  const stepper = () => useViewerNavStore.getState().stepper
  const open = (id: number) => act(() => useViewStore.getState().openPreview(id))
  return { view, stepper, open }
}

describe("gallery → viewer previous/next content", () => {
  it("T-ui-vnav-01 [Happy] stepping moves the active item like ArrowRight/ArrowLeft (selectOnly), and the viewer follows", async () => {
    // Arrange
    const { stepper, open } = await setup(fakeAssets(10))
    open(3)
    await waitFor(() => expect(stepper()).toMatchObject({ canPrev: true, canNext: true }))
    // Act 1
    act(() => stepper()?.step(1))
    // Assert 1
    await waitFor(() => expect(useViewStore.getState().previewId).toBe(4))
    expect([...useSelectionStore.getState().ids]).toEqual([4])
    // Act 2
    act(() => stepper()?.step(-1))
    act(() => stepper()?.step(-1))
    // Assert 2
    await waitFor(() => expect(useViewStore.getState().previewId).toBe(2))
    expect([...useSelectionStore.getState().ids]).toEqual([2])
  })

  it("T-ui-vnav-02 [Boundary] first item: canPrev false, canNext true; last item: the opposite; stepping past the end does nothing", async () => {
    // Arrange
    const { stepper, open } = await setup(fakeAssets(10))
    // Act + Assert: 첫 항목
    open(1)
    await waitFor(() => expect(stepper()).toMatchObject({ canPrev: false, canNext: true }))
    act(() => stepper()?.step(-1))
    expect(useViewStore.getState().previewId).toBe(1)
    // 마지막 항목
    open(10)
    await waitFor(() => expect(stepper()).toMatchObject({ canPrev: true, canNext: false }))
    act(() => stepper()?.step(1))
    expect(useViewStore.getState().previewId).toBe(10)
  })

  it("T-ui-vnav-03 [Boundary] a single result has nothing to navigate → no stepper (buttons are not drawn)", async () => {
    const { stepper, open } = await setup(fakeAssets(1))
    open(1)
    await waitFor(() => expect(useViewStore.getState().previewId).toBe(1))
    expect(stepper()).toBeNull()
  })

  it("T-ui-vnav-04 [Boundary] paged mode: next on the last item of a page opens the next page's first item, previous on its first item goes back to the last", async () => {
    // Arrange
    const { stepper, open, view } = await setup(fakeAssets(1000), "paged")
    open(100)
    await waitFor(() => expect(stepper()).toMatchObject({ canPrev: true, canNext: true }))
    const nav = screen.getByRole("navigation", { name: "쪽 이동" })
    // Act 1
    act(() => stepper()?.step(1))
    // Assert 1
    expect(await within(nav).findByText("101–200 / 1,000건")).toBeInTheDocument()
    await waitFor(() => expect(useViewStore.getState().previewId).toBe(101))
    expect([...useSelectionStore.getState().ids]).toEqual([101])
    // Act 2
    await waitFor(() => expect(stepper()?.canPrev).toBe(true))
    act(() => stepper()?.step(-1))
    // Assert 2
    expect(await within(nav).findByText("1–100 / 1,000건")).toBeInTheDocument()
    await waitFor(() => expect(useViewStore.getState().previewId).toBe(100))
    view.unmount()
  })

  it("T-ui-vnav-05 [Validation] an opened item that is not in the results (not opened from this gallery) gets no stepper", async () => {
    const { stepper, open } = await setup(fakeAssets(10))
    open(999)
    await act(async () => undefined)
    expect(stepper()).toBeNull()
  })

  it("T-ui-vnav-06 [Business rule] unmounting the gallery withdraws the stepper", async () => {
    const { stepper, open, view } = await setup(fakeAssets(10))
    open(3)
    await waitFor(() => expect(stepper()).not.toBeNull())
    view.unmount()
    expect(stepper()).toBeNull()
  })
})
