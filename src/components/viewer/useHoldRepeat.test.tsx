// Categories: Happy (T-ui-hold-01, 02) / Boundary (T-ui-hold-03 delay, 08 click) / Concurrency (T-ui-hold-04 one outstanding step) / Validation (T-ui-hold-06 non-primary) / Business rule (T-ui-hold-05 stop events, 07 unmount)
import { act, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { HOLD_DELAY_MS, HOLD_INTERVAL_MS, useHoldRepeat } from "./useHoldRepeat"

function Harness({ step }: { step: () => void | Promise<unknown> }) {
  const hold = useHoldRepeat(step)
  return <button type="button" {...hold}>step</button>
}

beforeEach(() => vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] }))
afterEach(() => vi.useRealTimers())

const advance = (ms: number) => act(async () => void (await vi.advanceTimersByTimeAsync(ms)))
const down = (el: Element, init: PointerEventInit = {}) => fireEvent.pointerDown(el, { button: 0, pointerId: 1, isPrimary: true, ...init })
const setup = (step: () => void | Promise<unknown> = vi.fn()) => {
  const view = render(<Harness step={step} />)
  return { step, view, btn: screen.getByRole("button", { name: "step" }) }
}

describe("useHoldRepeat", () => {
  it("T-ui-hold-01 [Happy] pointerdown steps once immediately", () => {
    const { step, btn } = setup()
    down(btn)
    expect(step).toHaveBeenCalledTimes(1)
  })

  it("T-ui-hold-03 [Boundary] no repeat before 400ms; repeats after, at the fixed cadence; stops on pointerup", async () => {
    // Arrange
    const { step, btn } = setup()
    down(btn)
    // Act + Assert: 399ms — 아직 1회
    await advance(HOLD_DELAY_MS - 1)
    expect(step).toHaveBeenCalledTimes(1)
    // 400ms — 반복 시작
    await advance(1)
    expect(step).toHaveBeenCalledTimes(2)
    await advance(HOLD_INTERVAL_MS)
    expect(step).toHaveBeenCalledTimes(3)
    // 손을 떼면 멈춘다
    fireEvent.pointerUp(btn)
    await advance(1000)
    expect(step).toHaveBeenCalledTimes(3)
  })

  it("T-ui-hold-04 [Concurrency] never more than one outstanding step — the next waits for the pending step (even the first one), and nothing runs after release", async () => {
    // Arrange: 끝나지 않는 IPC
    const pending: Array<() => void> = []
    const step = vi.fn(() => new Promise<void>((resolve) => pending.push(resolve)))
    const { btn } = setup(step)
    down(btn)
    // Act: 첫 이동이 400ms 지나도 안 끝났다 → 반복은 대기
    await advance(HOLD_DELAY_MS + 500)
    expect(step).toHaveBeenCalledTimes(1)
    // 첫 이동 완료 → 간격 뒤 두 번째
    await act(async () => pending.shift()?.())
    await advance(HOLD_INTERVAL_MS)
    expect(step).toHaveBeenCalledTimes(2)
    await advance(1000)
    expect(step).toHaveBeenCalledTimes(2)
    // 손을 뗀 뒤 완료돼도 다음을 요청하지 않는다
    fireEvent.pointerUp(btn)
    await act(async () => pending.shift()?.())
    await advance(1000)
    expect(step).toHaveBeenCalledTimes(2)
  })

  it("T-ui-hold-05 [Business rule] stops on pointerleave, pointercancel, lostpointercapture and window blur", async () => {
    for (const stop of [
      (el: Element) => fireEvent.pointerLeave(el),
      (el: Element) => fireEvent.pointerCancel(el),
      (el: Element) => fireEvent.lostPointerCapture(el),
      () => fireEvent.blur(window),
    ]) {
      // Arrange
      const { step, btn, view } = setup()
      down(btn)
      await advance(HOLD_DELAY_MS)
      expect(step).toHaveBeenCalledTimes(2)
      // Act
      stop(btn)
      await advance(1000)
      // Assert
      expect(step).toHaveBeenCalledTimes(2)
      view.unmount()
    }
  })

  it("T-ui-hold-06 [Validation] non-primary buttons and non-primary pointers are ignored", async () => {
    const { step, btn } = setup()
    down(btn, { button: 2 })
    down(btn, { isPrimary: false })
    await advance(2000)
    expect(step).not.toHaveBeenCalled()
  })

  it("T-ui-hold-07 [Business rule] unmounting while held clears the timers", async () => {
    const { step, btn, view } = setup()
    down(btn)
    view.unmount()
    await advance(2000)
    expect(step).toHaveBeenCalledTimes(1)
    expect(vi.getTimerCount()).toBe(0)
  })

  it("T-ui-hold-08 [Boundary] a full click (down, up, click) steps exactly once; a click without a pointer (keyboard/programmatic) steps once; leaving while held does not swallow the next click", async () => {
    // Arrange
    const { step, btn } = setup()
    // Act 1: 마우스 클릭
    down(btn)
    fireEvent.pointerUp(btn)
    fireEvent.click(btn)
    expect(step).toHaveBeenCalledTimes(1)
    // Act 2: 포인터 없는 click
    fireEvent.click(btn)
    expect(step).toHaveBeenCalledTimes(2)
    // Act 3: 누른 채 벗어남 → click 은 오지 않는다. 다음 프로그램 click 은 정상 동작
    down(btn)
    fireEvent.pointerLeave(btn)
    fireEvent.click(btn)
    expect(step).toHaveBeenCalledTimes(4)
  })
})
