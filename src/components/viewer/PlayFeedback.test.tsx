// Categories: Happy (T-ui-feedback-01, 05) / Boundary (T-ui-feedback-02, 03, 08) / Business rule (T-ui-feedback-04, 06) / Time (T-ui-feedback-07)
import { act, render, renderHook, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { PlayFeedbackOverlay } from "./PlayFeedbackOverlay"
import { SEEK_FEEDBACK_MS, TOGGLE_FEEDBACK_MS, movedBy, usePlayFeedback } from "./usePlayFeedback"

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

const advance = (ms: number) => act(() => void vi.advanceTimersByTime(ms))

describe("usePlayFeedback", () => {
  it("T-ui-feedback-01 [Happy] flashToggle shows the state after the input and hides after the duration", () => {
    // Arrange
    const { result } = renderHook(() => usePlayFeedback())
    // Act
    act(() => result.current.flashToggle(true))
    // Assert
    expect(result.current.toggle?.paused).toBe(true)
    advance(TOGGLE_FEEDBACK_MS - 1)
    expect(result.current.toggle).not.toBeNull()
    advance(1)
    expect(result.current.toggle).toBeNull()
  })

  it("T-ui-feedback-02 [Boundary] repeated toggles restart the animation (new tick) and extend the timer", () => {
    // Arrange
    const { result } = renderHook(() => usePlayFeedback())
    act(() => result.current.flashToggle(false))
    const first = result.current.toggle?.tick
    advance(400)
    // Act
    act(() => result.current.flashToggle(true))
    advance(400)
    // Assert
    expect(result.current.toggle?.tick).not.toBe(first)
    expect(result.current.toggle?.paused).toBe(true)
    advance(200)
    expect(result.current.toggle).toBeNull()
  })

  it("T-ui-feedback-03 [Boundary] flashSeek ignores 0 and non-finite amounts", () => {
    // Arrange
    const { result } = renderHook(() => usePlayFeedback())
    // Act
    act(() => {
      result.current.flashSeek(0)
      result.current.flashSeek(Number.NaN)
    })
    // Assert
    expect(result.current.seek).toBeNull()
  })

  it("T-ui-feedback-04 [Business rule] same direction accumulates, a direction change resets, and it hides after the last input", () => {
    // Arrange
    const { result } = renderHook(() => usePlayFeedback())
    // Act + Assert
    act(() => result.current.flashSeek(5))
    act(() => result.current.flashSeek(5))
    act(() => result.current.flashSeek(5))
    expect(result.current.seek).toMatchObject({ dir: 1, sec: 15 })
    act(() => result.current.flashSeek(-5))
    expect(result.current.seek).toMatchObject({ dir: -1, sec: 5 })
    advance(SEEK_FEEDBACK_MS - 1)
    expect(result.current.seek).not.toBeNull()
    advance(1)
    expect(result.current.seek).toBeNull()
    // 사라진 뒤 새 입력은 0 에서 다시 누적한다
    act(() => result.current.flashSeek(-5))
    expect(result.current.seek).toMatchObject({ dir: -1, sec: 5 })
  })

  it("T-ui-feedback-05 [Happy] each input restarts the 700ms hide timer", () => {
    // Arrange
    const { result } = renderHook(() => usePlayFeedback())
    act(() => result.current.flashSeek(5))
    advance(600)
    // Act
    act(() => result.current.flashSeek(5))
    advance(600)
    // Assert
    expect(result.current.seek?.sec).toBe(10)
  })

  it("T-ui-feedback-06 [Business rule] movedBy reports the clamped movement (0 at the start/end)", () => {
    // Arrange + Act + Assert
    expect(movedBy(10, 5, 100)).toBe(5)
    expect(movedBy(2, -5, 100)).toBe(-2)
    expect(movedBy(0, -5, 100)).toBe(0)
    expect(movedBy(100, 5, 100)).toBe(0)
    expect(movedBy(10, 5, Number.NaN)).toBe(5) // 길이를 모르면 위쪽은 막지 않는다
  })

  it("T-ui-feedback-07 [Time] unmount clears the timers", () => {
    // Arrange
    const { result, unmount } = renderHook(() => usePlayFeedback())
    act(() => {
      result.current.flashToggle(true)
      result.current.flashSeek(5)
    })
    expect(vi.getTimerCount()).toBe(2)
    // Act
    unmount()
    // Assert
    expect(vi.getTimerCount()).toBe(0)
  })
})

describe("PlayFeedbackOverlay", () => {
  it("T-ui-feedback-08 [Boundary] is aria-hidden, empty when idle, and shows state and the accumulated seconds text", () => {
    // Arrange + Act
    const view = render(<PlayFeedbackOverlay toggle={null} seek={null} />)
    // Assert
    const root = view.container.firstElementChild
    expect(root).toHaveAttribute("aria-hidden", "true")
    expect(root?.children).toHaveLength(0)
    view.rerender(<PlayFeedbackOverlay toggle={{ paused: true, tick: 1 }} seek={{ dir: -1, sec: 9.6, tick: 1 }} />)
    expect(view.container.querySelector('[data-slot="play-feedback-toggle"]')).toHaveAttribute("data-state", "paused")
    expect(screen.getByText("−10초")).toBeInTheDocument()
    view.rerender(<PlayFeedbackOverlay toggle={{ paused: false, tick: 2 }} seek={{ dir: 1, sec: 15, tick: 2 }} />)
    expect(view.container.querySelector('[data-slot="play-feedback-toggle"]')).toHaveAttribute("data-state", "playing")
    expect(screen.getByText("+15초")).toBeInTheDocument()
  })
})
