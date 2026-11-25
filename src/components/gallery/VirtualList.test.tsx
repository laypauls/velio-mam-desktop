// Categories: Boundary (T-ui-list-01) / Happy (T-ui-list-02)
import { act, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { useSelectionStore, useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { LIST_GRID_TEMPLATE, LIST_MIN_W } from "./VirtualList"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe("List view", () => {
  it("T-ui-list-01 [Boundary] renders the column headers and only a bounded number of rows for 100k assets", async () => {
    // Arrange
    stubLayout({ width: 1000, height: 600 })
    const api = createMockApi({ count: 100_000 })
    await api.jobSummary() // 합성 데이터 생성을 대기 시간 밖에서 끝낸다
    renderApp(api)
    // Act
    act(() => useViewStore.getState().setViewMode("list"))
    const options = await screen.findAllByRole("option", {}, { timeout: 10_000 })
    // Assert: 열 머리글 7개 중 텍스트 6개 (정렬 UI 는 없다 — 머리글은 aria-hidden 정적 텍스트)
    const box = screen.getByRole("listbox", { name: "미디어 목록" })
    for (const col of ["이름", "유형", "촬영일", "길이", "크기", "상태"]) expect(within(box).getByText(col, { ignore: "[role=option] *" })).toBeInTheDocument()
    expect(options.length).toBeGreaterThan(0)
    expect(options.length).toBeLessThanOrEqual(40) // 600px / 36px + overscan, 100,000 이 아니다
    const first = options[0]
    expect(within(first).getAllByText(/./).length).toBeGreaterThanOrEqual(5) // 이름·유형·촬영일·길이·크기·상태 셀
  })

  it("T-ui-list-02 [Happy] keyboard moves by one row, Left/Right are ignored, and PageDown jumps by the visible rows", async () => {
    // Arrange
    stubLayout({ width: 1000, height: 600 })
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 500 }))
    act(() => useViewStore.getState().setViewMode("list"))
    const options = await screen.findAllByRole("option")
    const box = screen.getByRole("listbox", { name: "미디어 목록" })
    await user.click(options[0])
    const ids = () => [...useSelectionStore.getState().ids]
    const first = Number(options[0].getAttribute("data-asset-id"))
    const second = Number(options[1].getAttribute("data-asset-id"))
    // Act + Assert
    await user.keyboard("{ArrowRight}")
    expect(ids()).toEqual([first]) // 좌우는 목록에서 무시
    await user.keyboard("{ArrowDown}")
    expect(ids()).toEqual([second])
    expect(box.getAttribute("aria-activedescendant")).toBe(`gallery-opt-${second}`)
    // 보이는 행 수 = floor((600 - 32) / 36) = 15
    await user.keyboard("{PageDown}")
    const target = screen.getAllByRole("option")[16] // 가상화 목록의 17번째 행이 존재해야 한다
    expect(target).toBeDefined()
    expect(ids()).toHaveLength(1)
    expect(box.getAttribute("aria-activedescendant")).toBe(`gallery-opt-${target.getAttribute("data-asset-id")}`)
  })
})

describe("List view narrow width (regression)", () => {
  it("T-ui-list-03 [Boundary] the name column keeps a real minimum and header, rows and wrapper share the min width so the list scrolls instead of collapsing", async () => {
    // Arrange: 좁은 목록 영역 (~480px)
    stubLayout({ width: 480, height: 600 })
    renderApp(createMockApi({ count: 200 }))
    act(() => useViewStore.getState().setViewMode("list"))
    const options = await screen.findAllByRole("option")
    const box = screen.getByRole("listbox", { name: "미디어 목록" })
    // Act
    const header = box.querySelector<HTMLElement>("[aria-hidden='true']:not([data-placeholder])")
    // Assert: 이름 열 템플릿은 0 으로 접히는 minmax(0,1fr) 이 아니다
    expect(LIST_GRID_TEMPLATE).toContain("minmax(10rem,1fr)")
    expect(LIST_GRID_TEMPLATE).not.toContain("minmax(0,1fr)")
    // 머리글·행·가상화 래퍼가 같은 최소 폭을 공유한다 (jsdom 은 레이아웃이 없어 클래스 계약으로 확인)
    expect(header).not.toBeNull()
    expect(header).toHaveClass(LIST_MIN_W)
    expect(options[0]).toHaveClass(LIST_MIN_W)
    expect(header?.parentElement).toHaveClass(LIST_MIN_W)
    // 머리글 글자는 그대로 렌더 (줄바꿈 방지는 whitespace-nowrap)
    expect(within(header as HTMLElement).getByText("이름")).toBeInTheDocument()
  })
})

describe("View mode toggle", () => {
  it("T-ui-list-04 [Happy] a 3-way toggle (gallery / list / filmstrip) switches the mode; the card display menu is enabled only in gallery", async () => {
    // Arrange
    stubLayout({ width: 1000, height: 600 })
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 200 }))
    await screen.findAllByRole("option")
    const group = screen.getByRole("radiogroup", { name: "보기 방식" })
    expect(within(group).getByRole("radio", { name: "갤러리" })).toBeChecked()
    // Act + Assert: 격자 → 목록
    await user.click(within(group).getByRole("radio", { name: "목록" }))
    expect(useViewStore.getState().viewMode).toBe("list")
    expect(screen.getByRole("button", { name: "표시 옵션" })).toBeDisabled()
    // 목록 → 격자
    await user.click(within(group).getByRole("radio", { name: "갤러리" }))
    expect(useViewStore.getState().viewMode).toBe("grid")
    expect(screen.getByRole("button", { name: "표시 옵션" })).toBeEnabled()
    // 눌린 항목을 다시 눌러도 해제되지 않는다
    await user.click(within(group).getByRole("radio", { name: "갤러리" }))
    expect(useViewStore.getState().viewMode).toBe("grid")
  })
})
