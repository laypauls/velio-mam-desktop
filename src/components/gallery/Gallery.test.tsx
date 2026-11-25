// Categories: Boundary (T-ui-gallery-01) / Business rule (-02) / Happy (-03) / External failure (-04)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary } from "@/lib/ipc-contract"
import { renderApp, stubLayout } from "@/test/layout"
import { optionNamed } from "@/test/queries"
import { mockFixedResults } from "@/test/searchMock"
import { useSelectionStore, useViewStore } from "@/stores"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const asset = (id: number, availability: AssetSummary["availability"]): AssetSummary => ({
  assetId: id,
  fileName: `file_${id}.jpg`,
  mediaType: "image",
  availability,
  width: 100,
  height: 100,
  durationMs: null,
  fileSize: 1000,
  capturedAt: Date.UTC(2026, 9, 1),
  modifiedAt: Date.UTC(2026, 9, 1),
  favorite: false,
  rating: 0,
  thumbState: "done",
  aiState: "none",
  title: null,
  revision: 1,
})

describe("Gallery", () => {
  it("T-ui-gallery-01 [Boundary] 100k assets render only visible cells and fetch only a few 100-item blocks", async () => {
    // Arrange
    stubLayout({ width: 1000, height: 600 })
    const api = createMockApi({ count: 100_000 })
    // 합성 데이터 생성(지연 생성)을 대기 시간 밖에서 끝낸다 — 병렬 빌드 부하에서 기본 1초 findBy 를 넘기지 않도록
    const { totalAssets } = await api.jobSummary()
    const spy = vi.spyOn(api, "searchAssets")
    // Act
    renderApp(api)
    // 첫 검색은 10만 건 메모리 필터·정렬이라 부하 시 1초를 넘을 수 있다 — 결정적 작업의 상한만 넓힌다
    const options = await screen.findAllByRole("option", {}, { timeout: 10_000 })
    // Assert
    // 보이는 블록(+앞뒤 1블록)만 받는다 — 100,000 건 전체가 아니라 100건 블록 소수
    expect(spy.mock.calls.length).toBeLessThanOrEqual(3)
    expect(spy.mock.calls.every(([req]) => req.limit === 100)).toBe(true)
    expect(spy.mock.calls[0][0].cursor).toBeNull()
    expect(options.length).toBeGreaterThan(0)
    expect(options.length).toBeLessThanOrEqual(60) // 100,000 이 아니라 보이는 행(+overscan)만
    expect(totalAssets).toBeGreaterThan(90_000)
  })

  it("T-ui-gallery-02 [Business rule] shows offline badge, hides missing by default, shows it after enabling the filter", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    const all = [asset(1, "online"), asset(2, "offline"), asset(3, "missing")]
    const { search: spy } = mockFixedResults(api, (r) =>
      all.filter((a) => (a.availability !== "missing" || r.filters.includeMissing) && (a.availability !== "offline" || r.filters.includeOffline))
    )
    const user = userEvent.setup()
    renderApp(api)
    // Act + Assert (기본)
    const list = await screen.findByRole("listbox", { name: "미디어 목록" })
    expect(spy.mock.calls[0][0].filters).toMatchObject({ includeOffline: true, includeMissing: false })
    expect(within(list).getAllByRole("option")).toHaveLength(2)
    // 상태는 카드 이름·배지·메타 줄에 모두 있다 — 카드의 접근 가능한 이름으로 확인한다
    expect(within(list).getByRole("option", { name: /오프라인/ })).toBeInTheDocument()
    expect(within(list).queryByRole("option", { name: /파일 없음/ })).not.toBeInTheDocument()
    // Act: 필터 켜기
    await user.click(screen.getByRole("button", { name: "필터" }))
    await user.click(await screen.findByRole("checkbox", { name: "파일 없음 포함" }))
    // Assert
    const widened = await screen.findByRole("listbox", { name: "미디어 목록" })
    expect(await within(widened).findByRole("option", { name: /파일 없음/ })).toBeInTheDocument()
  })

  it("T-ui-gallery-03 [Happy] click / ctrl / shift select and the detail panel follows the last clicked asset", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 500 }))
    const options = await screen.findAllByRole("option")
    // 카드 이름은 `파일명, 유형…` — 파일명만 떼어 낸다
    const names = options.map((o) => (o.getAttribute("aria-label") as string).split(", ")[0])
    const opt = (i: number) => screen.getByRole("option", { name: optionNamed(names[i]) })
    // Act + Assert: 단일 클릭
    await user.click(opt(0))
    expect(opt(0)).toHaveAttribute("aria-selected", "true")
    const panel = screen.getByRole("complementary", { name: "상세 정보 패널" })
    expect(await within(panel).findByRole("heading", { level: 2, name: names[0] })).toBeInTheDocument()
    // Ctrl 토글
    await user.keyboard("{Control>}")
    await user.click(opt(2))
    await user.keyboard("{/Control}")
    expect(opt(0)).toHaveAttribute("aria-selected", "true")
    expect(opt(2)).toHaveAttribute("aria-selected", "true")
    expect(opt(1)).toHaveAttribute("aria-selected", "false")
    expect(screen.getByText("2개 선택됨")).toBeInTheDocument()
    expect(await within(panel).findByRole("heading", { level: 2, name: names[2] })).toBeInTheDocument()
    // 일반 클릭 후 Shift 범위
    await user.click(opt(1))
    await user.keyboard("{Shift>}")
    await user.click(opt(4))
    await user.keyboard("{/Shift}")
    for (const i of [1, 2, 3, 4]) expect(opt(i)).toHaveAttribute("aria-selected", "true")
    expect(opt(0)).toHaveAttribute("aria-selected", "false")
    expect(screen.getByText("4개 선택됨")).toBeInTheDocument()
    // Ctrl+A (무한 스크롤): 메모리에 불러온 항목만 — 블록 0·1(200건). 선택 바가 그 사실을 알린다
    screen.getByRole("listbox", { name: "미디어 목록" }).focus()
    await user.keyboard("{Control>}a{/Control}")
    expect(await screen.findByText("200개 선택됨")).toBeInTheDocument()
    expect(screen.getByText("불러온 200개만 선택됨")).toBeInTheDocument()
  })

  it("T-ui-gallery-04 [External failure] search failure shows inline alert and retry recovers", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    vi.spyOn(api, "searchAssets").mockRejectedValueOnce({ code: "io", message: "boom" })
    const user = userEvent.setup()
    // Act
    renderApp(api)
    const alert = await screen.findByRole("alert")
    // Assert
    expect(alert).toHaveTextContent("검색 결과를 불러오지 못했습니다")
    expect(alert).toHaveTextContent("파일 입출력 중 오류")
    expect(screen.queryByRole("option")).not.toBeInTheDocument()
    // Act: 다시 시도
    await user.click(within(alert).getByRole("button", { name: "다시 시도" }))
    // Assert
    await waitFor(() => expect(screen.getAllByRole("option").length).toBeGreaterThan(0))
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
  })
})

describe("Gallery focus", () => {
  it("T-ui-galfocus-01 [Business rule] click then Esc keeps the selection, closes the preview and leaves focus on the listbox", async () => {
    // 컨테이너 전체에는 포커스 링을 그리지 않는다(클릭 뒤 키 입력으로 :focus-visible 이 켜져 갤러리 전체에 링이 생기던 버그) —
    // 키보드 위치는 커서 카드가 표시하므로 클래스가 아니라 선택·포커스 동작으로 검증한다
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    const api = createMockApi({ count: 50 })
    await api.updateSettings({ hideShortcut: null }) // 기본 Esc 는 숨기기 단축키 — 기존 Esc 동작은 단축키가 다른 키일 때 돌아온다
    renderApp(api)
    const list = await screen.findByRole("listbox", { name: "미디어 목록" })
    const [first] = await within(list).findAllByRole("option")
    // Act
    act(() => useViewStore.getState().openPreview(1)) // 보기 영역이 열려 있어야 Esc 로 닫힌다
    await user.click(first)
    expect(useViewStore.getState().previewId).not.toBeNull()
    const selected = [...useSelectionStore.getState().ids]
    await user.keyboard("{Escape}")
    // Assert
    expect(useViewStore.getState().previewId).toBeNull()
    expect([...useSelectionStore.getState().ids]).toEqual(selected)
    expect(selected).toHaveLength(1)
    expect(document.activeElement).toBe(list)
  })
})
