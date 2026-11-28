// Categories: Happy (T-ui-gensub-04) / Business rule (T-ui-gensub-04 position, T-ui-gensub-05 mpv interpolation) / Validation (T-ui-gensub-04 literal text)
import { act, render, renderHook, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { stubRafQueue } from "@/test/webgl"
import { GeneratedSubtitleOverlay } from "./GeneratedSubtitleOverlay"
import { MAX_EXTRAPOLATE_MS, useMpvSegment } from "./useGeneratedClock"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe("<GeneratedSubtitleOverlay>", () => {
  it("T-ui-gensub-04 [Business rule] file subtitle shown → top; generated only → bottom; no segment → renders nothing", () => {
    // Arrange
    const seg = { startMs: 0, endMs: 1000, text: "생성된 문장" }
    const view = render(<GeneratedSubtitleOverlay segment={seg} fileShown />)
    // Assert: 둘 다 → 위
    expect(screen.getByText("생성된 문장").closest("[data-slot=generated-subtitle]")).toHaveAttribute("data-position", "top")
    // Act: 파일 자막 없음 → 아래
    view.rerender(<GeneratedSubtitleOverlay segment={seg} fileShown={false} />)
    expect(screen.getByText("생성된 문장").closest("[data-slot=generated-subtitle]")).toHaveAttribute("data-position", "bottom")
    // Act: 구간 없음
    view.rerender(<GeneratedSubtitleOverlay segment={null} fileShown />)
    expect(screen.queryByText("생성된 문장")).not.toBeInTheDocument()
    expect(view.container).toBeEmptyDOMElement()
  })

  it("T-ui-gensub-04 [Validation] markup-like text is rendered literally (React text node, no HTML parsing)", () => {
    // Arrange + Act
    render(<GeneratedSubtitleOverlay segment={{ startMs: 0, endMs: 1, text: "<b>굵게</b> & 1 < 2" }} fileShown={false} />)
    // Assert
    expect(screen.getByText("<b>굵게</b> & 1 < 2")).toBeInTheDocument()
    expect(document.querySelector("b")).toBeNull()
  })
})

describe("useMpvSegment (position events are at most 5Hz → interpolated while playing)", () => {
  const SEGS = [
    { startMs: 1000, endMs: 1500, text: "A" },
    { startMs: 1500, endMs: 3000, text: "B" },
  ]

  function setup(initial: { timeMs: number; playing: boolean; rate: number }) {
    const raf = stubRafQueue()
    let now = 10_000
    vi.spyOn(performance, "now").mockImplementation(() => now)
    const hook = renderHook((p: { timeMs: number; playing: boolean; rate: number }) => useMpvSegment(SEGS, p.timeMs, p.playing, p.rate), { initialProps: initial })
    return { raf, hook, advance: (ms: number) => void (now += ms), tick: () => act(() => raf.flush()) }
  }

  it("T-ui-gensub-05 [Happy] playing: the segment changes between events from last position + elapsed time", async () => {
    // Arrange
    const t = setup({ timeMs: 1000, playing: true, rate: 1 })
    expect(t.hook.result.current?.text).toBe("A")
    // Act: 200ms 경과 (다음 이벤트 전) → 아직 A
    t.advance(200)
    await t.tick()
    expect(t.hook.result.current?.text).toBe("A")
    // Act: 520ms 경과 → 1520ms = B
    t.advance(320)
    await t.tick()
    // Assert
    expect(t.hook.result.current?.text).toBe("B")
  })

  it("T-ui-gensub-05 [Boundary] interpolation is capped, so a stalled event stream cannot run the subtitle ahead", async () => {
    // Arrange
    const t = setup({ timeMs: 1000, playing: true, rate: 1 })
    // Act: 이벤트 없이 한참 경과
    t.advance(60_000)
    await t.tick()
    // Assert: 1000 + 상한 = 1600 → B (그 이상 앞서가지 않는다)
    expect(MAX_EXTRAPOLATE_MS).toBe(600)
    expect(t.hook.result.current?.text).toBe("B")
  })

  it("T-ui-gensub-05 [Business rule] rate scales the elapsed time; a new event resets the base", async () => {
    // Arrange
    const t = setup({ timeMs: 1000, playing: true, rate: 2 })
    // Act: 300ms × 2 = 600ms → 1600
    t.advance(300)
    await t.tick()
    // Assert
    expect(t.hook.result.current?.text).toBe("B")
    // Act: 새 위치 이벤트 (탐색) → 기준 재설정
    t.hook.rerender({ timeMs: 1100, playing: true, rate: 2 })
    expect(t.hook.result.current?.text).toBe("A")
  })

  it("T-ui-gensub-05 [Business rule] paused: shows the event position only and schedules no frames", async () => {
    // Arrange
    const t = setup({ timeMs: 1200, playing: false, rate: 1 })
    // Assert
    expect(t.hook.result.current?.text).toBe("A")
    expect(t.raf.pending()).toBe(0)
    // Act: 일시정지 중 탐색
    t.hook.rerender({ timeMs: 2000, playing: false, rate: 1 })
    expect(t.hook.result.current?.text).toBe("B")
    // Act: 재생 → 일시정지 전환은 대기 중 프레임을 취소한다
    t.hook.rerender({ timeMs: 2000, playing: true, rate: 1 })
    expect(t.raf.pending()).toBe(1)
    t.hook.rerender({ timeMs: 2000, playing: false, rate: 1 })
    expect(t.raf.pending()).toBe(0)
  })
})
