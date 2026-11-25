// Categories: Happy (T-ui-rating-01, 02, 05) / Boundary (T-ui-rating-03, 04) / Business rule (T-ui-rating-06)
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { describe, expect, it, vi } from "vitest"
import "@/i18n"
import { RatingInput } from "./RatingInput"
import { RatingStars } from "./RatingStars"

function Harness({ initial = 0, onChange }: { initial?: number; onChange?: (v: number) => void }) {
  const [value, setValue] = useState(initial)
  return (
    <RatingInput
      value={value}
      onChange={(v) => {
        setValue(v)
        onChange?.(v)
      }}
    />
  )
}

describe("RatingStars (read-only)", () => {
  it("T-ui-rating-01 [Happy] exposes a single image with the accessible name 별점 N점", () => {
    // Arrange + Act
    render(<RatingStars value={4} />)
    // Assert
    expect(screen.getByRole("img", { name: "별점 4점" })).toBeInTheDocument()
    expect(screen.queryAllByRole("radio")).toHaveLength(0)
  })
})

describe("RatingInput", () => {
  it("T-ui-rating-02 [Happy] is a radiogroup of 5 radios; clicking a star sets that value", async () => {
    // Arrange
    const onChange = vi.fn()
    const user = userEvent.setup()
    render(<Harness onChange={onChange} />)
    expect(screen.getByRole("radiogroup", { name: "별점" })).toBeInTheDocument()
    expect(screen.getAllByRole("radio")).toHaveLength(5)
    // Act
    await user.click(screen.getByRole("radio", { name: "별점 3점" }))
    // Assert
    expect(onChange).toHaveBeenLastCalledWith(3)
    expect(screen.getByRole("radio", { name: "별점 3점" })).toBeChecked()
  })

  it("T-ui-rating-03 [Boundary] clicking the current star again clears the rating (0)", async () => {
    // Arrange
    const onChange = vi.fn()
    const user = userEvent.setup()
    render(<Harness initial={2} onChange={onChange} />)
    // Act
    await user.click(screen.getByRole("radio", { name: "별점 2점" }))
    // Assert
    expect(onChange).toHaveBeenLastCalledWith(0)
    expect(screen.queryByRole("radio", { checked: true })).not.toBeInTheDocument()
  })

  it("T-ui-rating-04 [Boundary] arrow keys change the value and stay within 0..5", async () => {
    // Arrange
    const user = userEvent.setup()
    render(<Harness initial={4} />)
    screen.getByRole("radio", { name: "별점 4점" }).focus()
    // Act + Assert
    await user.keyboard("{ArrowRight}")
    expect(screen.getByRole("radio", { name: "별점 5점" })).toBeChecked()
    await user.keyboard("{ArrowRight}") // 5 에서 더 올라가지 않는다
    expect(screen.getByRole("radio", { name: "별점 5점" })).toBeChecked()
    await user.keyboard("{ArrowLeft}{ArrowLeft}")
    expect(screen.getByRole("radio", { name: "별점 3점" })).toBeChecked()
    await user.keyboard("{ArrowLeft}{ArrowLeft}{ArrowLeft}{ArrowLeft}")
    expect(screen.queryByRole("radio", { checked: true })).not.toBeInTheDocument() // 0 = 미평가
  })

  it("T-ui-rating-05 [Validation] disabled input does not react to clicks", async () => {
    // Arrange
    const onChange = vi.fn()
    const user = userEvent.setup()
    render(<RatingInput value={1} onChange={onChange} disabled />)
    // Act
    await user.click(screen.getByRole("radio", { name: "별점 4점" }))
    // Assert
    expect(onChange).not.toHaveBeenCalled()
  })

  it("T-ui-rating-06 [Business rule] a click does not bubble to the parent (card select / open)", async () => {
    // Arrange
    const parent = vi.fn()
    const user = userEvent.setup()
    render(
      <div role="presentation" onClick={parent} onDoubleClick={parent}>
        <RatingInput value={0} onChange={() => undefined} />
      </div>
    )
    // Act
    await user.dblClick(screen.getByRole("radio", { name: "별점 2점" }))
    // Assert
    expect(parent).not.toHaveBeenCalled()
  })
})
