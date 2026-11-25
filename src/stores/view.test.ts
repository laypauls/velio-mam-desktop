// Categories: Business rule (T-ui-view-01, 02, 03)
import { beforeEach, describe, expect, it } from "vitest"
import { DEFAULT_CARD_DISPLAY } from "@/components/gallery/cardDisplay"
import { useViewStore } from "./view"

beforeEach(() => useViewStore.setState({ cardDisplay: DEFAULT_CARD_DISPLAY, cardDisplayRev: 0 }))

describe("view store card size", () => {
  it("T-ui-view-01 [Business rule] setCardSize changes only cardSize and bumps the rev (persistence trigger)", () => {
    // Arrange + Act
    useViewStore.getState().setCardSize("xl")
    // Assert
    const s = useViewStore.getState()
    expect(s.cardDisplay).toEqual({ ...DEFAULT_CARD_DISPLAY, cardSize: "xl" })
    expect(s.cardDisplayRev).toBe(1)
  })

  it("T-ui-view-02 [Business rule] resetCardDisplay restores the display options but keeps cardSize", () => {
    // Arrange
    const s = useViewStore.getState()
    s.setCardSize("xs")
    s.setCardDisplay({ check: "always", dateBy: "modified" })
    // Act
    useViewStore.getState().resetCardDisplay()
    // Assert
    expect(useViewStore.getState().cardDisplay).toEqual({ ...DEFAULT_CARD_DISPLAY, cardSize: "xs" })
    expect(useViewStore.getState().cardDisplayRev).toBe(3)
  })

  it("T-ui-view-03 [Business rule] applyCardPreset does not touch cardSize", () => {
    // Arrange
    useViewStore.getState().setCardSize("l")
    // Act
    useViewStore.getState().applyCardPreset("off")
    // Assert
    expect(useViewStore.getState().cardDisplay.cardSize).toBe("l")
  })
})
