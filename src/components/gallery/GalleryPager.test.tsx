// Categories: Happy (T-ui-gwin-90) / Boundary (T-ui-gwin-91, 92)
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"
import "@/i18n"
import { GalleryPager, pageNumbers } from "./GalleryPager"

describe("pageNumbers", () => {
  it("T-ui-gwin-90 [Happy] keeps first/last and the current page +-2, with ellipses between", () => {
    expect(pageNumbers(123, 10_000, false)).toEqual([0, "gap", 121, 122, 123, 124, 125, "gap", 9_999])
    expect(pageNumbers(0, 5, false)).toEqual([0, 1, 2, "gap", 4])
  })

  it("T-ui-gwin-91 [Boundary] an estimated total never lists the last page and ends with an ellipsis", () => {
    expect(pageNumbers(0, 10, true)).toEqual([0, 1, 2, "gap"])
    expect(pageNumbers(9, 10, true)).toEqual([0, "gap", 7, 8, 9])
  })
})

describe("GalleryPager", () => {
  it("T-ui-gwin-92 [Boundary] a single page disables every move and the last-page button is present for exact totals", async () => {
    // Arrange
    const onPage = vi.fn()
    render(<GalleryPager page={0} pageCount={1} range={[0, 30]} total={30} totalIsEstimate={false} onPage={onPage} />)
    // Act
    await userEvent.click(screen.getByRole("button", { name: "다음 쪽" }))
    // Assert
    expect(screen.getByRole("button", { name: "마지막 쪽" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "첫 쪽" })).toBeDisabled()
    expect(screen.getByText("1–30 / 30건")).toBeInTheDocument()
    expect(onPage).not.toHaveBeenCalled()
  })
})
