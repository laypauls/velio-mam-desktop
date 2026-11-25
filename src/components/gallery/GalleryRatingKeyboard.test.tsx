// Categories: Happy (T-ui-rating-30, 31) / Business rule (T-ui-rating-32, 33, 34)
// 갤러리 listbox 에서 1~5 = 별점 지정, 0 = 해제 (D5)
import { act, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary } from "@/lib/ipc-contract"
import { useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { optionNamed } from "@/test/queries"
import { fakeAssets, mockFixedResults } from "@/test/searchMock"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup(items: AssetSummary[] = fakeAssets(6)) {
  stubLayout({ width: 1000, height: 600 })
  const api = createMockApi({ count: 10 })
  mockFixedResults(api, items)
  const setRating = vi.spyOn(api, "setRating")
  const user = userEvent.setup()
  renderApp(api)
  act(() => useViewStore.getState().setCardSize("xl"))
  await screen.findAllByRole("option")
  const box = screen.getByRole("listbox", { name: "미디어 목록" })
  const opt = (id: number) => screen.getByRole("option", { name: optionNamed(items.find((i) => i.assetId === id)!.fileName) })
  return { user, box, opt, setRating }
}

describe("gallery rating shortcuts", () => {
  it("T-ui-rating-30 [Happy] pressing 3 rates the selected item", async () => {
    // Arrange
    const { user, opt, box, setRating } = await setup()
    await user.click(opt(1))
    expect(box).toHaveFocus()
    // Act
    await user.keyboard("3")
    // Assert
    await waitFor(() => expect(setRating).toHaveBeenCalledWith([1], 3))
  })

  it("T-ui-rating-31 [Happy] 0 clears, and the whole multi-selection is rated", async () => {
    // Arrange
    const { user, opt, setRating } = await setup()
    await user.click(opt(1))
    await user.keyboard("{Control>}")
    await user.click(opt(2))
    await user.keyboard("{/Control}")
    // Act
    await user.keyboard("0")
    // Assert
    await waitFor(() => expect(setRating).toHaveBeenCalledTimes(1))
    const [ids, rating] = setRating.mock.calls[0]
    expect([...ids].sort()).toEqual([1, 2])
    expect(rating).toBe(0)
  })

  it("T-ui-rating-32 [Business rule] ignored while typing in the search input", async () => {
    // Arrange
    const { user, setRating, opt } = await setup()
    await user.click(opt(1)) // 선택이 있는 상태에서도
    const input = screen.getByRole("combobox", { name: "검색" })
    // Act
    await user.click(input)
    await user.keyboard("4")
    // Assert
    expect(input).toHaveValue("4")
    expect(setRating).not.toHaveBeenCalled()
  })

  it("T-ui-rating-33 [Business rule] ignored with modifier keys held", async () => {
    // Arrange
    const { user, opt, setRating } = await setup()
    await user.click(opt(1))
    // Act
    await user.keyboard("{Control>}3{/Control}")
    await user.keyboard("{Alt>}2{/Alt}")
    // Assert
    expect(setRating).not.toHaveBeenCalled()
  })

  it("T-ui-rating-34 [Boundary] digits above 5 and a rating with nothing selected do nothing", async () => {
    // Arrange
    const { user, box, setRating, opt } = await setup()
    await user.click(opt(1))
    // Act
    await user.keyboard("6")
    await user.keyboard("9")
    // Assert
    expect(box).toHaveFocus()
    expect(setRating).not.toHaveBeenCalled()
  })
})
