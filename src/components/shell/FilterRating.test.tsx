// Categories: Happy (T-ui-rating-20) / Boundary (T-ui-rating-21) / Validation (T-ui-rating-22) / Business rule (T-ui-rating-23)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import * as ipc from "@/lib/ipc-metadata"
import { resetAllStores, useSearchStore } from "@/stores"
import { FilterPopover } from "./FilterPopover"
import { ANY, fromSelectValue, normalizeRatingRange, toSelectValue } from "./FilterRatingRow"

vi.mock("@/lib/ipc-metadata")

beforeEach(() => {
  resetAllStores()
  vi.mocked(ipc.listMetadataFields).mockResolvedValue([])
})
afterEach(() => vi.restoreAllMocks())

async function open() {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  const user = userEvent.setup()
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <FilterPopover />
    </QueryClientProvider>
  )
  await user.click(screen.getByRole("button", { name: "필터" }))
  const pick = async (trigger: string, option: string) => {
    await user.click(screen.getByRole("combobox", { name: trigger }))
    await user.click(within(await screen.findByRole("listbox")).getByRole("option", { name: option }))
  }
  return { user, pick }
}
const filters = () => useSearchStore.getState().filters

describe("FilterPopover rating row", () => {
  it("T-ui-rating-20 [Happy] picking min and max stars applies ratingMin / ratingMax as one active filter", async () => {
    // Arrange
    const { pick } = await open()
    // Act
    await pick("최소 별점", "★3")
    await pick("최대 별점", "★5")
    // Assert
    expect(filters()).toMatchObject({ ratingMin: 3, ratingMax: 5 })
    expect(screen.getByLabelText("적용된 필터 1개")).toBeInTheDocument()
  })

  it("T-ui-rating-21 [Boundary] 미평가 is 0 (not null) and 제한 없음 returns to null", async () => {
    // Arrange
    const { pick } = await open()
    // Act + Assert
    await pick("최대 별점", "미평가")
    expect(filters()).toMatchObject({ ratingMin: null, ratingMax: 0 })
    await pick("최대 별점", "제한 없음")
    expect(filters().ratingMax).toBeNull()
  })

  it("T-ui-rating-22 [Validation] min greater than max is swapped", async () => {
    // Arrange
    const { pick } = await open()
    await pick("최대 별점", "★2")
    // Act: 최소를 최대보다 크게
    await pick("최소 별점", "★4")
    // Assert
    expect(filters()).toMatchObject({ ratingMin: 2, ratingMax: 4 })
  })

  it("T-ui-rating-23 [Business rule] the sentinel is never an empty string; helpers map sentinel <-> null and swap", () => {
    expect(ANY).not.toBe("")
    expect(toSelectValue(null)).toBe(ANY)
    expect(toSelectValue(0)).toBe("0")
    expect(fromSelectValue(ANY)).toBeNull()
    expect(fromSelectValue("0")).toBe(0)
    expect(normalizeRatingRange(5, 1)).toEqual([1, 5])
    expect(normalizeRatingRange(null, 1)).toEqual([null, 1])
    expect(normalizeRatingRange(2, 2)).toEqual([2, 2])
  })
})
