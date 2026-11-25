// Categories: Business rule, gate ON (T-ui-rail-40, 41, 42, 58) — 설정(indexRail / galleryPaging)이 실제 갤러리 레일에 반영되는지
import { QueryClient } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import App from "@/App"
import { createMockApi } from "@/lib/api/mock"
import type { SearchIndex, SettingsPatch } from "@/lib/ipc-contract"
import { resetAllStores } from "@/stores"
import { stubLayout } from "@/test/layout"
import { optionNamed } from "@/test/queries"
import { fakeAssets, mockFixedResults } from "@/test/searchMock"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function renderGallery(patch: SettingsPatch) {
  stubLayout({ width: 1000, height: 600 })
  const api = createMockApi({ count: 10 })
  await api.updateSettings(patch)
  mockFixedResults(api, [...fakeAssets(60, 1, 8, 2026), ...fakeAssets(60, 61, 2, 2025)])
  resetAllStores()
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
  render(<App api={api} queryClient={qc} />)
  await screen.findAllByRole("option")
  return screen.getByRole("scrollbar", { name: "목차 · 스크롤 위치" })
}

describe("index rail in the gallery", () => {
  it("T-ui-rail-40 [Happy] auto: the rail carries the year labels of the loaded index", async () => {
    // Arrange + Act
    const rail = await renderGallery({ indexRail: "auto" })
    // Assert
    expect(within(rail).getByText("2026")).toBeInTheDocument()
    expect(within(rail).getByText("2025")).toBeInTheDocument()
  })

  it("T-ui-rail-58 [Business rule] aria-controls references the gallery scroll area, and the rail is not in the tab order", async () => {
    // Arrange + Act
    const rail = await renderGallery({ indexRail: "auto" })
    // Assert
    const id = rail.getAttribute("aria-controls")
    expect(id).toBeTruthy()
    expect(document.getElementById(id!)).toBe(screen.getByRole("listbox", { name: "미디어 목록" }))
    expect(rail).not.toHaveAttribute("tabindex")
  })

  it("T-ui-rail-41 [Business rule, gate ON] indexRail=off hides the labels but the position handle stays", async () => {
    // Arrange + Act
    const rail = await renderGallery({ indexRail: "off" })
    // Assert
    expect(rail).toHaveAttribute("aria-valuenow", "0")
    expect(within(rail).queryByText("2026")).not.toBeInTheDocument()
    expect(within(rail).queryByText("2025")).not.toBeInTheDocument()
  })

  it("T-ui-rail-42 [Business rule, gate ON] galleryPaging=paged shows the pager and the rail reports the page", async () => {
    // Arrange + Act
    const rail = await renderGallery({ galleryPaging: "paged", galleryPageSize: 50 })
    // Assert
    expect(screen.getByRole("navigation", { name: "쪽 이동" })).toBeInTheDocument()
    expect(rail).toHaveAttribute("aria-valuetext", expect.stringContaining("1쪽"))
    expect(screen.getByTestId("rail-page-span")).toBeInTheDocument()
  })
})

// 이름·제목 정렬(색인 종류 initial/none)은 구간 머리글 없는 하나의 격자 — 레일 위치는 구간의 첫 항목 위치로 환산돼야 한다
async function renderUngrouped(patch: SettingsPatch, index: Pick<SearchIndex, "kind" | "sections">, opts: { height?: number } = {}) {
  const total = index.sections.reduce((a, x) => a + x.count, 0)
  stubLayout({ width: 1000, height: opts.height ?? 600 })
  const api = createMockApi({ count: 10 })
  await api.updateSettings(patch)
  const { index: spy, search } = mockFixedResults(api, fakeAssets(total))
  spy.mockImplementation(async () => ({ total, generation: 1, ...index }))
  resetAllStores()
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
  render(<App api={api} queryClient={qc} />)
  await screen.findAllByRole("option")
  const rail = screen.getByRole("scrollbar", { name: "목차 · 스크롤 위치" })
  const labelTops = (text: string) => within(rail).getAllByText(text).map((e) => parseFloat(e.style.top))
  const labels = () => [...within(screen.getByTestId("rail-ticks")).queryAllByText(/./)].map((e) => ({ text: e.textContent ?? "", top: parseFloat(e.style.top) }))
  return { rail, labelTops, labels, search }
}
const sec = (spec: Record<string, number>): SearchIndex["sections"] => Object.entries(spec).map(([key, count]) => ({ key, label: null, count }))
// 실제 앱 재현(제목 오름차순, 자산 30개): 구간이 한 행보다 작고 끝 쪽 구간은 맨 위에 올 수 없다
const REPRO = sec({ sym: 3, a: 1, c: 5, g: 1, l: 2, m: 1, o: 1, p: 1, s: 5, t: 3, v: 1, ㄱ: 1, ㅁ: 1, ㅇ: 1, ㅍ: 1, ㅎ: 2 })
const REPRO_LABELS = ["#", "A", "C", "G", "L", "M", "O", "P", "S", "T", "V", "ㄱ", "ㅁ", "ㅇ", "ㅍ", "ㅎ"]
const INITIALS: SearchIndex["sections"] = ["sym", "a", "b", "ㄱ", "etc_end"].map((key) => ({ key, label: null, count: 24 }))

describe("index rail with ungrouped layouts (name/title sorts)", () => {
  it("T-ui-rail-43 [Happy] infinite: every initial section gets its own increasing position; valuetext follows the top item and a jump", async () => {
    // Arrange
    const { rail, labelTops } = await renderUngrouped({ indexRail: "always" }, { kind: "initial", sections: INITIALS })
    // Assert: 라벨 5개가 모두 있고 위에서 아래로 늘어난다
    const tops = ["#", "A", "B", "ㄱ", "기타"].map((t) => labelTops(t)[0])
    expect(tops.every((t) => Number.isFinite(t))).toBe(true)
    for (let i = 1; i < tops.length; i++) expect(tops[i]).toBeGreaterThan(tops[i - 1])
    expect(rail).toHaveAttribute("aria-valuetext", "#")
    // Act: B 라벨 위치 클릭 → 그 구간 머리로
    fireEvent.mouseDown(rail, { clientY: tops[2] + 6, button: 0 })
    fireEvent.mouseUp(window, { clientY: tops[2] + 6 })
    // Assert
    await waitFor(() => expect(rail).toHaveAttribute("aria-valuetext", "B"))
    expect(screen.getByRole("option", { name: /file_49.jpg/ })).toBeInTheDocument()
  })

  it("T-ui-rail-44 [Happy] paged: sections map to their global share, the jump lands on the page of the section", async () => {
    // Arrange
    const { rail, labelTops } = await renderUngrouped({ indexRail: "always", galleryPaging: "paged", galleryPageSize: 50 }, { kind: "initial", sections: INITIALS })
    const tops = ["#", "A", "B", "ㄱ", "기타"].map((t) => labelTops(t)[0])
    for (let i = 1; i < tops.length; i++) expect(tops[i]).toBeGreaterThan(tops[i - 1])
    expect(rail).toHaveAttribute("aria-valuetext", "# · 1쪽")
    // Act: ㄱ(전역 72번째) → 2쪽
    fireEvent.mouseDown(rail, { clientY: tops[3] + 6, button: 0 })
    fireEvent.mouseUp(window, { clientY: tops[3] + 6 })
    // Assert
    await waitFor(() => expect(rail).toHaveAttribute("aria-valuetext", expect.stringMatching(/^ㄱ · 2쪽$/)))
  })

  it("T-ui-rail-45 [Boundary] kind none (size/duration sorts): no labels, percentage text, a click seeks by fraction", async () => {
    // Arrange
    const { rail } = await renderUngrouped({ indexRail: "always" }, { kind: "none", sections: [{ key: "all", label: null, count: 120 }] })
    expect(rail).toHaveAttribute("aria-valuetext", "0%")
    // Act
    fireEvent.mouseDown(rail, { clientY: 300, button: 0 })
    fireEvent.mouseUp(window, { clientY: 300 })
    // Assert
    await waitFor(() => expect(Number(rail.getAttribute("aria-valuenow"))).toBeGreaterThan(40))
  })
})

describe("name index: equal-spacing slots (contacts-app style)", () => {
  const expectEvenlySpaced = (tops: number[]) => {
    const gaps = tops.slice(1).map((t, i) => t - tops[i])
    expect(Math.min(...gaps)).toBeGreaterThan(0)
    expect(Math.max(...gaps) - Math.min(...gaps)).toBeLessThan(0.01)
  }

  it("T-ui-rail-46 [Happy] a short list with 16 tiny sections shows all 16 labels in order at equal spacing, including the Korean consonants", async () => {
    // Arrange + Act
    const { labels } = await renderUngrouped({ indexRail: "always" }, { kind: "initial", sections: REPRO })
    // Assert
    expect(labels().map((l) => l.text)).toEqual(REPRO_LABELS)
    expectEvenlySpaced(labels().map((l) => l.top))
  })

  it("T-ui-rail-47 [Boundary] descending order keeps the backend order of the slots (ㅎ at the top, # at the bottom)", async () => {
    // Arrange + Act
    const { labels, rail } = await renderUngrouped({ indexRail: "always" }, { kind: "initial", sections: [...REPRO].reverse() })
    // Assert
    expect(labels().map((l) => l.text)).toEqual([...REPRO_LABELS].reverse())
    expectEvenlySpaced(labels().map((l) => l.top))
    expect(rail).toHaveAttribute("aria-valuetext", "ㅎ")
  })

  it("T-ui-rail-48 [Happy] clicking the ㅇ slot goes to that section even when it sits in the last screenful", async () => {
    // Arrange
    const { rail, labels } = await renderUngrouped({ indexRail: "always" }, { kind: "initial", sections: REPRO })
    expect(rail).toHaveAttribute("aria-valuetext", "#")
    const top = labels().find((l) => l.text === "ㅇ")!.top
    // Act: 라벨에서 조금 벗어난 자리를 눌러도 가장 가까운 칸
    fireEvent.mouseDown(rail, { clientY: top + 5, button: 0 })
    fireEvent.mouseUp(window, { clientY: top + 5 })
    // Assert
    await waitFor(() => expect(rail).toHaveAttribute("aria-valuetext", "ㅇ"))
  })

  it("T-ui-rail-49 [Boundary] a short track keeps the first section of each script group (#, A, 기타, ㄱ) and the last one", async () => {
    // Arrange + Act: 칸 간격 8px < 최소 14px
    const sections = sec({ sym: 4, a: 4, c: 4, etc: 4, ㄱ: 4, ㅎ: 4, etc_end: 4 })
    const { labels } = await renderUngrouped({ indexRail: "always" }, { kind: "initial", sections }, { height: 60 })
    // Assert
    expect(labels().map((l) => l.text)).toEqual(["#", "A", "기타", "ㄱ", "기타"])
    const tops = labels().map((l) => l.top)
    expect([...tops].sort((a, b) => a - b)).toEqual(tops)
  })

  it("T-ui-rail-50 [Boundary] a 16-section index on a 120px track still names # A ㄱ ㅎ and never lists more labels than fit", async () => {
    // Arrange + Act
    const { labels } = await renderUngrouped({ indexRail: "always" }, { kind: "initial", sections: REPRO }, { height: 120 })
    // Assert
    const texts = labels().map((l) => l.text)
    for (const must of ["#", "A", "ㄱ", "ㅎ"]) expect(texts).toContain(must)
    expect(texts.length).toBeLessThan(16)
  })

  it("T-ui-rail-51 [Happy] the handle moves monotonically (interpolating between slots) while scrolling through one section", async () => {
    // Arrange: 가운데 큰 구간(A)의 앞뒤에 작은 구간
    const { rail } = await renderUngrouped({ indexRail: "always" }, { kind: "initial", sections: sec({ sym: 2, a: 96, b: 2 }) })
    const box = screen.getByRole("listbox", { name: "미디어 목록" })
    const seen: number[] = [Number(rail.getAttribute("aria-valuenow"))]
    // Act: 한 줄(약 1행)씩 — 렌더는 act 안에서 동기로 끝난다
    for (let i = 0; i < 14; i++) {
      fireEvent.wheel(box, { deltaY: 250 })
      seen.push(Number(rail.getAttribute("aria-valuenow")))
    }
    // Assert: 줄어들지 않고, A 칸(50%)에서 B 칸(99%) 사이를 여러 단계로 지난다
    for (let i = 1; i < seen.length; i++) expect(seen[i]).toBeGreaterThanOrEqual(seen[i - 1])
    const inA = seen.filter((v) => v > 50 && v < 99)
    expect(new Set(inA).size).toBeGreaterThan(3)
  })
})

describe("wheel over the rail", () => {
  it("T-ui-rail-52 [Happy] infinite: the wheel scrolls the gallery like over the viewport, prevents page scroll, and requests the new block", async () => {
    // Arrange
    const { rail, search } = await renderUngrouped({ indexRail: "auto" }, { kind: "initial", sections: sec({ sym: 2500, a: 2500 }) })
    expect(screen.getByRole("option", { name: optionNamed("file_1.jpg") })).toBeInTheDocument()
    search.mockClear()
    // Act: 한 번에 멀리 (빠른 스크롤 → 멈춘 뒤 블록 요청)
    const notPrevented = fireEvent.wheel(rail, { deltaY: 100_000 })
    // Assert
    expect(notPrevented).toBe(false)
    await waitFor(() => expect(screen.queryByRole("option", { name: optionNamed("file_1.jpg") })).not.toBeInTheDocument())
    await waitFor(() => expect(search).toHaveBeenCalled())
    expect(screen.getByTestId("rail-ticks")).toHaveAttribute("data-state", "visible")
  })

  it("T-ui-rail-53 [Boundary] line-mode wheel deltas are converted like the viewport (deltaMode 1)", async () => {
    // Arrange
    const { rail } = await renderUngrouped({ indexRail: "auto" }, { kind: "initial", sections: sec({ sym: 60, a: 60 }) })
    // Act: 줄 15 x 40px = 600px — 첫 행(248.8)은 화면 밖
    fireEvent.wheel(rail, { deltaY: 15, deltaMode: 1 })
    // Assert
    await waitFor(() => expect(screen.queryByRole("option", { name: optionNamed("file_1.jpg") })).not.toBeInTheDocument())
  })

  it("T-ui-rail-54 [Business rule, gate ON] paged: the wheel over the rail scrolls within the current page", async () => {
    // Arrange
    const { rail } = await renderUngrouped({ indexRail: "auto", galleryPaging: "paged", galleryPageSize: 50 }, { kind: "initial", sections: sec({ sym: 60, a: 60 }) })
    expect(screen.getByRole("option", { name: optionNamed("file_1.jpg") })).toBeInTheDocument()
    // Act
    fireEvent.wheel(rail, { deltaY: 700 })
    // Assert
    await waitFor(() => expect(screen.queryByRole("option", { name: optionNamed("file_1.jpg") })).not.toBeInTheDocument())
    expect(rail).toHaveAttribute("aria-valuetext", expect.stringContaining("1쪽"))
  })
})
