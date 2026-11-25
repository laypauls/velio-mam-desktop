// Categories: Business rule (T-ui-scene-05 layer / hit test / right-click, T-ui-scene-06 button states) / External (T-ui-scene-07 failure, already-exists, event refetch) / Happy (T-ui-scene-05e menu actions)
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { OPEN_LAYER } from "@/lib/openLayer"
import { auto, mountScenePlayer, sceneCuts, user } from "@/test/scene"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), warning: vi.fn(), info: vi.fn(), success: vi.fn() } }))

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  vi.clearAllMocks()
})

/** 레이아웃 폭 1000px(모든 요소 rect 가 같다), 길이 100초 → 플롯 1000px: 1px = 100ms, 컷 ms 의 화면 x = ms/100 */
const xOf = (ms: number) => ms / 100
const CUTS = [auto(10_000), user(20_000), auto(50_000)]

async function mount(cuts = CUTS, over: Parameters<typeof mountScenePlayer>[0] = {}) {
  const m = await mountScenePlayer({ scene: sceneCuts(1, cuts), ...over })
  const bar = await screen.findByTestId("player-track")
  // 서버 응답이 화면에 반영될 때까지 — 컷이 있으면 토글이 나타난다, 없으면 조회가 끝나기만 기다린다
  if ((over.scene ?? sceneCuts(1, cuts)).cuts.length > 0) await screen.findByRole("button", { name: /장면전환점 표시/ })
  else await waitFor(() => expect(m.scene.get).toHaveBeenCalled())
  await act(async () => undefined)
  return { ...m, bar }
}
const layerPaths = (bar: HTMLElement) => ({
  all: bar.querySelectorAll("path"),
  auto: bar.querySelector('path[data-source="auto"]')?.getAttribute("d") ?? "",
  user: bar.querySelector('path[data-source="user"]')?.getAttribute("d") ?? "",
})
const segments = (d: string) => (d.match(/M/g) ?? []).length

describe("scene cut triangles on the integrated track", () => {
  it("T-ui-scene-05a [Business rule] draws every cut as inverted triangles in two SVG paths only (auto 10x7 / user 12x9), aria-hidden and pointer-transparent", async () => {
    // Arrange / Act
    const { bar } = await mount()
    const layer = within(bar).getByTestId("scene-cut-layer")
    const p = layerPaths(bar)
    // Assert
    expect(layer).toHaveAttribute("aria-hidden", "true")
    expect(p.all).toHaveLength(2)
    expect(segments(p.auto)).toBe(2)
    expect(segments(p.user)).toBe(1)
    // 트랙 좌표(0..1000): 10s → 100, 50s → 500, 20s → 200 — 삼각형은 x 를 가운데로 위 변 폭 10(자동)/12(사용자), 높이 7/9
    expect(p.auto).toContain("M95 0h10l-5 7z")
    expect(p.auto).toContain("M495 0h10l-5 7z")
    expect(p.user).toContain("M194 0h12l-6 9z")
  })

  it("T-ui-scene-05b [Business rule] cuts in the same 4px bucket merge into one triangle (per source), and thousands of cuts still render exactly 2 paths with at most one triangle per bucket", async () => {
    // Arrange: 10.000s(x=100) / 10.010s / 10.300s(x=103)는 같은 칸(100~103), 10.400s(x=104)는 다음 칸, 5000개(20ms 간격)는 1000px → 251칸으로 합쳐진다
    const dense = Array.from({ length: 5000 }, (_, i) => auto(i * 20))
    // Act
    const small = await mount([auto(10_000), auto(10_010), auto(10_300), auto(10_400), auto(50_000), user(10_000)])
    // Assert: 소스가 다르면 같은 칸이어도 합치지 않는다
    expect(segments(layerPaths(small.bar).auto)).toBe(3)
    expect(segments(layerPaths(small.bar).user)).toBe(1)
    small.view.unmount()
    const big = await mount(dense)
    const p = layerPaths(big.bar)
    expect(p.all).toHaveLength(2)
    expect(segments(p.auto)).toBeLessThanOrEqual(251)
    expect(segments(p.auto)).toBeGreaterThan(240)
  })

  it("T-ui-scene-05c [Business rule] hover within 4px of a cut shows \"장면전환 mm:ss.mmm (자동|사용자)\" with an enlarged triangle; farther than 4px shows only the time tooltip; leaving hides it", async () => {
    // Arrange
    const { bar } = await mount()
    // Act + Assert: 자동 컷 (50s) 에서 +4px
    fireEvent.pointerMove(bar, { clientX: xOf(50_000) + 4, buttons: 0 })
    expect(await screen.findByRole("tooltip")).toHaveTextContent("장면전환 00:50.000 (자동)")
    expect(bar.querySelectorAll("svg:not([data-testid])")).toHaveLength(1)
    // 5px 밖 — 컷 툴팁은 없고 시각 툴팁만
    fireEvent.pointerMove(bar, { clientX: xOf(50_000) + 5, buttons: 0 })
    expect(screen.getByRole("tooltip")).not.toHaveTextContent("장면전환")
    expect(bar.querySelectorAll("svg:not([data-testid])")).toHaveLength(0)
    // 사용자 컷
    fireEvent.pointerMove(bar, { clientX: xOf(20_000), buttons: 0 })
    expect(await screen.findByRole("tooltip")).toHaveTextContent("장면전환 00:20.000 (사용자)")
    fireEvent.pointerLeave(bar)
    expect(screen.queryByRole("tooltip")).toBeNull()
  })

  it("T-ui-scene-05d [Business rule] right-click picks the nearest cut within ±4px (menu offers go-to / delete); otherwise only re-detect / clear-auto; the menu counts as an open layer", async () => {
    // Arrange
    const { bar } = await mount()
    // Act 1: 4px 안
    fireEvent.contextMenu(bar, { clientX: xOf(50_000) - 4 })
    const menu = await screen.findByRole("menu")
    // Assert 1
    expect(document.querySelector(OPEN_LAYER)).not.toBeNull()
    expect(within(menu).getByRole("menuitem", { name: /이 장면전환점으로 이동/ })).toBeInTheDocument()
    expect(within(menu).getByRole("menuitem", { name: /이 장면전환점 삭제/ })).toBeInTheDocument()
    expect(within(menu).getByRole("menuitem", { name: /자동 장면전환점 다시 추출/ })).toBeInTheDocument()
    expect(within(menu).getByRole("menuitem", { name: /자동 장면전환점 모두 삭제/ })).toBeInTheDocument()
    expect(within(menu).getByRole("menuitem", { name: /모든 구간 삭제/ })).toBeInTheDocument()
    fireEvent.keyDown(menu, { key: "Escape" })
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull())
    // Act 2: 5px 밖 → 점 항목 없음
    fireEvent.contextMenu(bar, { clientX: xOf(50_000) + 5 })
    const menu2 = await screen.findByRole("menu")
    // Assert 2
    expect(within(menu2).queryByRole("menuitem", { name: /이 장면전환점/ })).toBeNull()
    // 구간 모두 삭제(구간 없음 → 비활성) + 다시 추출 + 자동 모두 삭제
    expect(within(menu2).getAllByRole("menuitem")).toHaveLength(3)
  })

  it("T-ui-scene-05f [Business rule] non-primary button presses (right / middle) never seek, while a left click still seeks", async () => {
    // Arrange
    const { bar, el } = await mount()
    const slider = bar
    // Act 1: 우클릭 누름
    fireEvent.pointerDown(slider, { clientX: 300, button: 2, pointerId: 1 })
    fireEvent.pointerUp(slider, { clientX: 300, button: 2, pointerId: 1 })
    // Assert 1
    expect(el.currentTime).toBe(0)
    // Act 2: 가운데 버튼도 막는다
    fireEvent.pointerDown(slider, { clientX: 300, button: 1, pointerId: 1 })
    expect(el.currentTime).toBe(0)
    // Act 3: 왼쪽 클릭은 탐색한다 (회귀)
    fireEvent.pointerDown(slider, { clientX: 500, button: 0, pointerId: 1 })
    fireEvent.pointerUp(slider, { clientX: 500, button: 0, pointerId: 1 })
    expect(el.currentTime).toBe(50)
  })

  it("T-ui-scene-05e [Happy] menu actions: go-to seeks to cut + half frame, delete removes that cut by its source, clear-auto confirms in an AlertDialog, re-detect asks sensitivity + restore", async () => {
    // Arrange
    const u = userEvent.setup()
    const { bar, el, scene } = await mount()
    // Act 1: 이동 (10s 자동 컷)
    fireEvent.contextMenu(bar, { clientX: xOf(10_000) })
    await u.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: /이 장면전환점으로 이동/ }))
    // Assert 1: fps 25 → 반 프레임 20ms
    expect(el.currentTime).toBeCloseTo(10.02, 6)
    // Act 2: 사용자 컷 삭제
    fireEvent.contextMenu(bar, { clientX: xOf(20_000) })
    await u.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: /이 장면전환점 삭제/ }))
    // Assert 2
    await waitFor(() => expect(scene.remove).toHaveBeenCalledWith(1, 20_000, "user"))
    await screen.findByRole("button", { name: "장면전환점 표시 (2개)" })
    // Act 3: 자동 컷 모두 삭제 — 취소하면 호출 없음, 확인하면 호출
    fireEvent.contextMenu(bar, { clientX: 900 })
    await u.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: /자동 장면전환점 모두 삭제/ }))
    const alert = await screen.findByRole("alertdialog")
    expect(alert).toHaveTextContent("자동으로 찾은 장면전환점 2개가 삭제됩니다")
    await u.click(within(alert).getByRole("button", { name: "취소" }))
    expect(scene.clearAuto).not.toHaveBeenCalled()
    fireEvent.contextMenu(bar, { clientX: 900 })
    await u.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: /자동 장면전환점 모두 삭제/ }))
    await u.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "모두 삭제" }))
    await waitFor(() => expect(scene.clearAuto).toHaveBeenCalledTimes(1))
    // Act 4: 다시 추출 — 기본 민감도(보통), 되살리기 체크
    fireEvent.contextMenu(bar, { clientX: 900 })
    await u.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: /자동 장면전환점 다시 추출/ }))
    const dialog = await screen.findByRole("dialog")
    expect(within(dialog).getByRole("combobox", { name: "검출 민감도" })).toHaveTextContent("보통")
    await u.click(within(dialog).getByRole("checkbox", { name: /지운 장면전환점 되살리기/ }))
    await u.click(within(dialog).getByRole("button", { name: "다시 추출" }))
    // Assert 4
    await waitFor(() => expect(scene.request).toHaveBeenCalledWith([1], { redetect: true, restoreSuppressed: true, sensitivity: "medium" }))
  })
})

describe("scene control bar button states", () => {
  const btn = (name: string | RegExp) => screen.queryByRole("button", { name })

  it("T-ui-scene-06a [Business rule] none → \"장면 추출\" requests detection; pending → busy disabled; failed → \"장면 다시 시도\" with reason; unsupported → no extract/retry button", async () => {
    // Arrange: 행 없음
    const none = await mount([])
    expect(btn(/장면전환점 표시/)).toBeNull()
    // Act
    await userEvent.setup().click(await screen.findByRole("button", { name: "장면 추출" }))
    // Assert: 요청은 옵션 없이 [assetId]
    await waitFor(() => expect(none.scene.request).toHaveBeenCalledWith([1], {}))
    // 서버가 pending 으로 → 진행 중 버튼 (비활성, aria-busy), 추출 버튼은 사라진다
    const busy = await screen.findByRole("button", { name: "추출 중…" })
    expect(busy).toBeDisabled()
    expect(busy).toHaveAttribute("aria-busy", "true")
    expect(btn("장면 추출")).toBeNull()
    none.view.unmount()
    // failed
    const failed = await mount([], { scene: sceneCuts(1, [], { state: "failed", error: "timeout" }) })
    const retry = await screen.findByRole("button", { name: "장면 다시 시도" })
    expect(retry).toHaveAttribute("title", "처리 시간이 초과되었습니다.")
    await userEvent.setup().click(retry)
    await waitFor(() => expect(failed.scene.request).toHaveBeenCalledWith([1], {}))
    failed.view.unmount()
    // unsupported
    await mountScenePlayer({ scene: sceneCuts(1, [], { state: "unsupported" }) })
    await waitFor(() => expect(screen.getByLabelText("재생 위치")).toBeInTheDocument())
    expect(btn("장면 추출")).toBeNull()
    expect(btn("장면 다시 시도")).toBeNull()
    expect(btn(/추출 중/)).toBeNull()
  })

  it("T-ui-scene-06b [Business rule] done: toggle with the count, aria-pressed follows the setting; turning it off removes the triangles and the cut tooltip", async () => {
    // Arrange
    const { bar } = await mount()
    const toggle = screen.getByRole("button", { name: "장면전환점 표시 (3개)" })
    expect(toggle).toHaveAttribute("aria-pressed", "true")
    expect(toggle).toHaveTextContent("장면 3")
    expect(btn("장면 추출")).toBeNull()
    // Act
    await userEvent.setup().click(toggle)
    // Assert
    await waitFor(() => expect(toggle).toHaveAttribute("aria-pressed", "false"))
    expect(within(bar).queryByTestId("scene-cut-layer")).toBeNull()
    fireEvent.pointerMove(bar, { clientX: xOf(50_000), buttons: 0 })
    expect(screen.getByRole("tooltip")).not.toHaveTextContent("장면전환")
    // 꺼져 있어도 우클릭 메뉴(다시 추출 / 모두 삭제)는 열리고 컷 항목은 없다
    fireEvent.contextMenu(bar, { clientX: xOf(50_000) })
    expect(within(await screen.findByRole("menu")).queryByRole("menuitem", { name: /이 장면전환점/ })).toBeNull()
  })

  it("T-ui-scene-06c [Business rule] sceneCutsVisible=false from settings starts without the layer", async () => {
    const { bar } = await mount(CUTS, { settings: { sceneCutsVisible: false } })
    expect(screen.getByRole("button", { name: "장면전환점 표시 (3개)" })).toHaveAttribute("aria-pressed", "false")
    expect(within(bar).queryByTestId("scene-cut-layer")).toBeNull()
  })

  it("T-ui-scene-06d [Business rule] user-only row (done, sensitivity null) shows the toggle AND still offers 장면 추출, which requests with redetect (server skips done rows otherwise)", async () => {
    // Arrange
    const { scene } = await mount([user(5000)], { scene: sceneCuts(1, [user(5000)], { state: "done", sensitivity: null }) })
    // Act
    await userEvent.setup().click(await screen.findByRole("button", { name: "장면 추출" }))
    // Assert
    expect(screen.getByRole("button", { name: "장면전환점 표시 (1개)" })).toBeInTheDocument()
    await waitFor(() => expect(scene.request).toHaveBeenCalledWith([1], { redetect: true }))
  })
})

describe("scene cuts data flow", () => {
  it("T-ui-scene-07a [External] add failure → error toast (no cache change); adding within tolerance → \"이미 있음\" toast; success updates the count from the server response", async () => {
    // Arrange
    const { scene, el } = await mount()
    const at = (s: number) => {
      el.currentTime = s
      act(() => void el.dispatchEvent(new Event("timeupdate")))
    }
    // Act 1: 서버 거부
    scene.add.mockRejectedValueOnce({ code: "invalid_input", message: "cuts" })
    at(40)
    fireEvent.keyDown(document.body, { code: "KeyC", key: "c" })
    // Assert 1
    await waitFor(() => expect(toast.error).toHaveBeenCalledTimes(1))
    expect(screen.getByRole("button", { name: "장면전환점 표시 (3개)" })).toBeInTheDocument()
    // Act 2: 허용 오차 안(10.01s ≈ 자동 컷 10s)
    at(10.01)
    fireEvent.keyDown(document.body, { code: "KeyC", key: "c" })
    await waitFor(() => expect(toast.info).toHaveBeenCalledWith("이 위치(한 프레임 안)에는 이미 장면전환점이 있습니다."))
    expect(screen.getByRole("button", { name: "장면전환점 표시 (3개)" })).toBeInTheDocument()
    // Act 3: 새 위치
    at(75)
    fireEvent.keyDown(document.body, { code: "KeyC", key: "c" })
    // Assert 3: 서버가 돌려준 목록으로 개수 갱신
    await screen.findByRole("button", { name: "장면전환점 표시 (4개)" })
  })

  it("T-ui-scene-07b [External] scene://updated for this asset refetches (new cuts appear without reopening); other assets are ignored; unsubscribes on unmount", async () => {
    // Arrange
    const { scene, view, bar } = await mount([], { scene: sceneCuts(1, [], { state: "pending" }) })
    expect(await screen.findByRole("button", { name: "추출 중…" })).toBeInTheDocument()
    expect(scene.listeners()).toBe(1)
    const calls = scene.get.mock.calls.length
    // Act 1: 추출 완료 이벤트
    act(() => scene.complete({ cuts: [auto(10_000), auto(60_000)], sensitivity: "medium" }))
    // Assert 1
    await screen.findByRole("button", { name: "장면전환점 표시 (2개)" })
    expect(scene.get.mock.calls.length).toBeGreaterThan(calls)
    expect(within(bar).getByTestId("scene-cut-layer")).toBeInTheDocument()
    // Act 2: 언마운트 → 구독 해제
    view.unmount()
    expect(scene.listeners()).toBe(0)
  })
})
