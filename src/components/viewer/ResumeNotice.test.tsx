// Categories: Happy (text, restart) / Time (3s auto-hide) / Boundary (null)
import { act, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { RESUME_NOTICE_MS, ResumeNotice } from "./ResumeNotice"

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

describe("ResumeNotice (D10)", () => {
  it("T-ui-resume-16 [Happy] shows '{time} 부터 이어서 재생' with a 처음부터 button, and disappears after 3s", () => {
    // Arrange + Act
    render(<ResumeNotice restoredMs={754_000} onRestart={() => undefined} />)
    // Assert
    expect(screen.getByRole("status")).toHaveTextContent("12:34 부터 이어서 재생")
    expect(screen.getByRole("button", { name: "처음부터" })).toBeInTheDocument()
    act(() => void vi.advanceTimersByTime(RESUME_NOTICE_MS - 1))
    expect(screen.getByRole("status")).toBeInTheDocument()
    act(() => void vi.advanceTimersByTime(1))
    expect(screen.queryByRole("status")).toBeNull()
  })

  it("T-ui-resume-17 [Happy] 처음부터 calls onRestart once, hides the notice and does not bubble the click to the stage", () => {
    // Arrange
    const onRestart = vi.fn()
    const onStageClick = vi.fn()
    render(
      <div onClick={onStageClick}>
        <ResumeNotice restoredMs={754_000} onRestart={onRestart} />
      </div>
    )
    // Act
    fireEvent.click(screen.getByRole("button", { name: "처음부터" }))
    // Assert
    expect(onRestart).toHaveBeenCalledTimes(1)
    expect(onStageClick).not.toHaveBeenCalled()
    expect(screen.queryByRole("status")).toBeNull()
  })

  it("T-ui-resume-18 [Boundary] renders nothing when nothing was restored", () => {
    // Arrange + Act
    render(<ResumeNotice restoredMs={null} onRestart={() => undefined} />)
    // Assert
    expect(screen.queryByRole("status")).toBeNull()
  })
})
