// Categories: Business rule (T-ui-ord-07, 08, 10, T-ui-sort-05) / Happy (T-ui-ord-09, 09b) / Boundary (T-ui-ord-09c)
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { useSearchStore, useSelectionStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup() {
  stubLayout({ width: 1000, height: 600 })
  // 응답 지연을 고정해 "보기 전환 직후 이전 목록이 남아 있는 구간"을 매번 재현한다 (지연 0 이면 CPU 부하에 따라 간헐적으로만 드러난다)
  const api = createMockApi({ count: 300, delayMs: 40 })
  const reorder = vi.spyOn(api, "reorderCollection")
  const user = userEvent.setup()
  renderApp(api)
  await screen.findAllByRole("option")
  return { api, reorder, user }
}

const enterCollection = async () => {
  act(() => useSearchStore.getState().setView({ kind: "collection", collectionId: 1 }))
  await screen.findByRole("heading", { name: "포트폴리오" })
  // 제목은 보기 전환 즉시 나타나지만 갤러리는 새 색인이 올 때까지 이전 목록(placeholder)을 그대로 보여 주다가 교체한다.
  // 그 사이에 잡은 요소는 곧 분리돼 클릭이 허공에 가므로, 컬렉션 순서 목록이 실제로 그려진 뒤(#1 배지 = 새 항목)에만 돌려준다
  await screen.findByText("#1")
  await waitFor(() => expect(screen.getByRole("listbox", { name: "미디어 목록" })).not.toHaveAttribute("aria-busy"))
  await waitFor(() => expect(screen.getAllByRole("option").length).toBeGreaterThan(3))
  return screen.getAllByRole("option")
}
/** 컬렉션 순서가 아닌 정렬(또는 라이브러리)의 새 목록이 그려질 때까지 — 정렬 전환 사이의 이전 목록·로딩 골격을 건너뛴다 */
const listWithoutOrderBadges = async () => {
  await waitFor(() => expect(screen.queryByText("#1")).not.toBeInTheDocument())
  await waitFor(() => expect(screen.getByRole("listbox", { name: "미디어 목록" })).not.toHaveAttribute("aria-busy"))
  await waitFor(() => expect(screen.getAllByRole("option").length).toBeGreaterThan(3))
}
const idOf = (el: HTMLElement) => Number(el.getAttribute("data-asset-id"))
const dt = () => ({ dataTransfer: { setData: vi.fn(), effectAllowed: "", dropEffect: "" } })

describe("collection order in the gallery", () => {
  it("T-ui-ord-07 [Business rule] 순서 이동 is enabled in collection order; with another sort its items are disabled and the reason is shown", async () => {
    // Arrange
    const { user } = await setup()
    const options = await enterCollection()
    fireEvent.contextMenu(options[1])
    await user.click(await screen.findByRole("menuitem", { name: /^순서 이동/ }))
    // Assert: 컬렉션 순서에서는 켜짐
    expect(await screen.findByRole("menuitem", { name: /^맨 앞으로/ })).not.toHaveAttribute("aria-disabled", "true")
    await user.keyboard("{Escape}{Escape}")
    // Act: 다른 정렬
    act(() => useSearchStore.getState().setSort({ field: "name", direction: "asc" }))
    await listWithoutOrderBadges()
    fireEvent.contextMenu(screen.getAllByRole("option")[1])
    await user.click(await screen.findByRole("menuitem", { name: /^순서 이동/ }))
    // Assert
    expect(await screen.findByRole("menuitem", { name: /^맨 앞으로/ })).toHaveAttribute("aria-disabled", "true")
    expect(screen.getByText(/컬렉션 순서\\?"일 때만/)).toBeInTheDocument()
  })

  it("T-ui-ord-08 [Business rule] Ctrl+Shift+Home moves the selection to the start in a collection; outside a collection it does nothing", async () => {
    // Arrange
    const { reorder, user } = await setup()
    const options = await enterCollection()
    const id = idOf(options[3])
    await user.click(options[3])
    // Act
    await user.keyboard("{Control>}{Shift>}{Home}{/Shift}{/Control}")
    // Assert
    await waitFor(() => expect(reorder).toHaveBeenCalledWith(1, [id], { kind: "start" }))
    // Act: 라이브러리로 나가면 호출되지 않는다
    reorder.mockClear()
    act(() => useSearchStore.getState().setView({ kind: "all" }))
    await waitFor(() => expect(screen.queryByRole("heading", { name: "포트폴리오" })).not.toBeInTheDocument())
    // 라이브러리 목록이 실제로 그려진 뒤(#n 배지 사라짐, 갱신 표시 해제)의 항목만 쓴다 — 이전 목록 요소를 누르면 클릭이 허공에 가서 단언이 공허해진다
    await listWithoutOrderBadges()
    const listbox = screen.getByRole("listbox", { name: "미디어 목록" })
    const target = (await screen.findAllByRole("option"))[2]
    await user.click(target)
    expect(target).toHaveAttribute("aria-selected", "true") // 클릭이 실제로 선택에 닿았다
    await user.keyboard("{Control>}{Shift>}{Home}{/Shift}{/Control}")
    expect(reorder).not.toHaveBeenCalled()
    expect(listbox).toBeInTheDocument()
  })

  it("T-ui-ord-08b [Business rule] the shortcut is consumed with a hint (no call) when the sort is not collection order", async () => {
    // Arrange
    const info = vi.spyOn(toast, "info")
    const { reorder, user } = await setup()
    const options = await enterCollection()
    act(() => useSearchStore.getState().setSort({ field: "name", direction: "asc" }))
    await listWithoutOrderBadges()
    await user.click((await screen.findAllByRole("option"))[1])
    void options
    // Act
    await user.keyboard("{Control>}{Shift>}{ArrowDown}{/Shift}{/Control}")
    // Assert
    expect(reorder).not.toHaveBeenCalled()
    expect(info).toHaveBeenCalledWith("정렬이 \"컬렉션 순서\"일 때만 순서를 바꿀 수 있습니다.")
  })

  it("T-ui-ord-09 [Happy] dragging a card onto another moves it before that card; the selection moves together", async () => {
    // Arrange
    const { reorder } = await setup()
    const options = await enterCollection()
    const [a, b, c] = [options[0], options[1], options[4]]
    expect(a).toHaveAttribute("draggable", "true")
    act(() => useSelectionStore.getState().selectAll([idOf(a), idOf(b)]))
    // Act
    fireEvent.dragStart(a, dt())
    fireEvent.dragOver(c, { ...dt(), clientY: 100 })
    fireEvent.drop(c, dt())
    // Assert
    await waitFor(() => expect(reorder).toHaveBeenCalledWith(1, [idOf(a), idOf(b)], { kind: "before", assetId: idOf(c) }))
  })

  it("T-ui-ord-09b [Happy] dropping on empty space moves to the end; dropping onto itself does nothing", async () => {
    // Arrange
    const { reorder } = await setup()
    const options = await enterCollection()
    const a = options[2]
    // Act 1: 자기 자신 위
    fireEvent.dragStart(a, dt())
    fireEvent.dragOver(a, dt())
    fireEvent.drop(a, dt())
    expect(reorder).not.toHaveBeenCalled()
    // Act 2: 빈 곳 (카드를 감싼 래퍼)
    fireEvent.dragStart(a, dt())
    const wrapper = a.closest("[role=listbox]")!.firstElementChild as HTMLElement
    fireEvent.dragOver(wrapper, dt())
    fireEvent.drop(wrapper, dt())
    // Assert
    await waitFor(() => expect(reorder).toHaveBeenCalledWith(1, [idOf(a)], { kind: "end" }))
  })

  it("T-ui-ord-09c [Boundary] auto-scroll runs on a stubbed rAF only during the drag and is cancelled on drop; no draggable cards in other sorts", async () => {
    // Arrange: rAF 를 직접 제어한다
    const frames = new Map<number, FrameRequestCallback>()
    let next = 1
    const { reorder } = await setup()
    const options = await enterCollection()
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => (frames.set(next, cb), next++))
    const cancel = vi.fn((id: number) => void frames.delete(id))
    vi.stubGlobal("cancelAnimationFrame", cancel)
    const a = options[1]
    // Act: 드래그 시작 → 루프 예약, 가장자리 근처(y=5) → 한 프레임 실행해도 오류 없이 다음 프레임 예약
    fireEvent.dragStart(a, dt())
    expect(frames.size).toBe(1)
    fireEvent.dragOver(options[2], { ...dt(), clientY: 5 })
    act(() => {
      const run = [...frames.values()]
      frames.clear()
      run.forEach((cb) => cb(0))
    })
    expect(frames.size).toBe(1)
    // 놓으면 취소
    fireEvent.drop(options[2], dt())
    expect(cancel).toHaveBeenCalled()
    expect(frames.size).toBe(0)
    await waitFor(() => expect(reorder).toHaveBeenCalled())
    // 다른 정렬에서는 드래그 불가
    act(() => useSearchStore.getState().setSort({ field: "name", direction: "asc" }))
    await waitFor(() => expect(screen.getAllByRole("option")[0]).toHaveAttribute("draggable", "false"))
  })

  it("T-ui-ord-10 [Business rule] #n badges appear only while sorted by collection order", async () => {
    // Arrange + Act
    await setup()
    await enterCollection()
    // Assert
    expect(screen.getByText("#1")).toBeInTheDocument()
    expect(screen.getByText("#2")).toBeInTheDocument()
    // Act: 다른 정렬
    act(() => useSearchStore.getState().setSort({ field: "name", direction: "asc" }))
    await listWithoutOrderBadges()
  })

  it("T-ui-sort-05 [Business rule] the sort dropdown offers 컬렉션 순서 only in a collection view, and its direction button is disabled there", async () => {
    // Arrange
    Element.prototype.hasPointerCapture ??= () => false
    Element.prototype.scrollIntoView ??= () => undefined
    const { user } = await setup()
    // Act 1: 라이브러리 — 항목 없음
    await user.click(screen.getByRole("combobox", { name: "정렬 기준" }))
    expect(within(await screen.findByRole("listbox", { name: "" }).catch(() => document.body)).queryByRole("option", { name: "컬렉션 순서" })).not.toBeInTheDocument()
    await user.keyboard("{Escape}")
    // Act 2: 컬렉션
    await enterCollection()
    // Assert
    expect(screen.getByRole("combobox", { name: "정렬 기준" })).toHaveTextContent("컬렉션 순서")
    expect(screen.getByRole("button", { name: /^정렬 방향/ })).toBeDisabled()
  })
})

describe("following moved items", () => {
  async function listSetup() {
    const t = await setup()
    await t.user.click(screen.getByRole("radio", { name: "목록" }))
    const options = await enterCollection()
    const header = await screen.findByText(/^\d+개$/)
    const total = Number(header.textContent!.replace("개", ""))
    return { ...t, options, total }
  }
  const visible = (n: number) => screen.queryByText(`#${n}`)

  it("T-ui-ord-15 [Happy] moving to the end scrolls to the last item; moving to the start scrolls back to the top (menu/shortcut path)", async () => {
    // Arrange
    const { user, reorder, options, total } = await listSetup()
    expect(total).toBeGreaterThan(26)
    expect(visible(total)).not.toBeInTheDocument()
    await user.click(options[0])
    // Act 1: 맨 뒤로 (단축키)
    await user.keyboard("{Control>}{Shift>}{End}{/Shift}{/Control}")
    // Assert 1
    await waitFor(() => expect(reorder).toHaveBeenCalledWith(1, expect.any(Array), { kind: "end" }))
    await waitFor(() => expect(visible(total)).toBeInTheDocument())
    expect(visible(1)).not.toBeInTheDocument()
    // Act 2: 보이는 마지막 항목을 맨 앞으로 (메뉴)
    const all = screen.getAllByRole("option")
    const bottom = all[all.length - 1]
    await user.click(bottom)
    fireEvent.contextMenu(bottom)
    await user.click(await screen.findByRole("menuitem", { name: /^순서 이동/ }))
    fireEvent.click(await screen.findByRole("menuitem", { name: /^맨 앞으로/ })) // 서브메뉴는 pointer 이동 시 jsdom 이 닫는다
    // Assert 2
    await waitFor(() => expect(visible(1)).toBeInTheDocument())
  })

  it("T-ui-ord-16 [Business rule] drag and drop does not scroll the gallery", async () => {
    // Arrange
    const { reorder, options, total } = await listSetup()
    // Act: 위쪽 카드를 보이는 다른 카드 앞으로 드롭
    fireEvent.dragStart(options[0], dt())
    fireEvent.dragOver(options[5], dt())
    fireEvent.drop(options[5], dt())
    await waitFor(() => expect(reorder).toHaveBeenCalled())
    // Assert: 여전히 맨 위
    expect(visible(1)).toBeInTheDocument()
    expect(visible(total)).not.toBeInTheDocument()
  })
})
