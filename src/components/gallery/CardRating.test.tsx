// Categories: Happy (T-ui-rating-40, 41) / Business rule (T-ui-rating-42, 43)
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary, GalleryCardDisplay } from "@/lib/ipc-contract"
import { AssetCell } from "./AssetCell"
import { DEFAULT_CARD_DISPLAY } from "./cardDisplay"

afterEach(() => vi.restoreAllMocks())

const base: AssetSummary = {
  assetId: 7,
  fileName: "photo.jpg",
  mediaType: "image",
  availability: "online",
  width: 1920,
  height: 1080,
  durationMs: null,
  fileSize: 1_000_000,
  capturedAt: Date.UTC(2026, 7, 2, 12),
  modifiedAt: Date.UTC(2026, 9, 20, 12),
  favorite: false,
  rating: 0,
  thumbState: "pending",
  aiState: "none",
  title: null,
  revision: 1,
}

function setup(asset: Partial<AssetSummary> = {}, display: GalleryCardDisplay = DEFAULT_CARD_DISPLAY) {
  const onSetRating = vi.fn()
  const onToggleFavorite = vi.fn()
  render(
    <ApiProvider api={createMockApi({ count: 1 })}>
      <div role="listbox" aria-label="목록" tabIndex={0}>
        <AssetCell
          asset={{ ...base, ...asset }}
          selected={false}
          display={display}
          cellWidth={240}
          selecting={false}
          onSelect={vi.fn()}
          onOpen={vi.fn()}
          onToggleFavorite={onToggleFavorite}
          onSetRating={onSetRating}
        />
      </div>
    </ApiProvider>
  )
  return { user: userEvent.setup(), onSetRating, onToggleFavorite }
}

describe("card rating", () => {
  it("T-ui-rating-40 [Happy] a rated card shows read-only stars in the info area; an unrated card shows none", () => {
    // Arrange + Act
    setup({ rating: 4 })
    // Assert
    expect(screen.getByRole("img", { name: "별점 4점" })).toBeInTheDocument()
  })

  it("T-ui-rating-41 [Boundary] unrated card has no read-only stars", () => {
    setup({ rating: 0 })
    expect(screen.queryByRole("img", { name: /별점/ })).not.toBeInTheDocument()
  })

  it("T-ui-rating-42 [Happy] the hover overlay input rates the item; the current star clears it", async () => {
    // Arrange
    const { user, onSetRating } = setup({ rating: 2 })
    // Act
    await user.click(screen.getByRole("radio", { name: "별점 5점" }))
    await user.click(screen.getByRole("radio", { name: "별점 2점" }))
    // Assert
    expect(onSetRating).toHaveBeenNthCalledWith(1, 7, 5)
    expect(onSetRating).toHaveBeenNthCalledWith(2, 7, 0)
  })

  it("T-ui-rating-43 [Business rule] display.rating off removes both the stars and the overlay input; the heart keeps its labels", () => {
    // Arrange + Act
    setup({ rating: 3 }, { ...DEFAULT_CARD_DISPLAY, rating: false })
    // Assert
    expect(screen.queryByRole("img", { name: /별점/ })).not.toBeInTheDocument()
    expect(screen.queryByRole("radiogroup")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "즐겨찾기에 추가" })).toBeInTheDocument()
  })
})
