// Categories: Happy (T-ui-width-01) / Boundary (T-ui-width-02)
import { act, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { useElementWidth } from "./useElementWidth"

let fire: (width: number) => void = () => undefined

class StubResizeObserver {
  constructor(cb: ResizeObserverCallback) {
    fire = (width) => cb([{ contentRect: { width } } as ResizeObserverEntry], this as unknown as ResizeObserver)
  }
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

afterEach(() => vi.unstubAllGlobals())

function setup(initial: number) {
  vi.stubGlobal("ResizeObserver", StubResizeObserver)
  const el = document.createElement("div")
  vi.spyOn(el, "clientWidth", "get").mockReturnValue(initial)
  return renderHook(() => useElementWidth(el))
}

describe("useElementWidth", () => {
  it("T-ui-width-01 [Happy] reads the initial width and follows resizes", () => {
    // Arrange
    const { result } = setup(800)
    expect(result.current).toBe(800)
    // Act
    act(() => fire(640.4))
    // Assert
    expect(result.current).toBe(640)
  })

  it("T-ui-width-02 [Boundary] a 0-width measurement (hidden element) keeps the last valid width", () => {
    // Arrange
    const { result } = setup(800)
    // Act: 숨김 → 0, 다시 표시 → 원래 폭
    act(() => fire(0))
    // Assert
    expect(result.current).toBe(800)
    act(() => fire(900))
    expect(result.current).toBe(900)
  })

  it("T-ui-width-03 [Boundary] a hidden element at mount (clientWidth 0) reports 0 until it gets a real width", () => {
    // Arrange + Act
    const { result } = setup(0)
    // Assert
    expect(result.current).toBe(0)
    act(() => fire(500))
    expect(result.current).toBe(500)
  })
})
