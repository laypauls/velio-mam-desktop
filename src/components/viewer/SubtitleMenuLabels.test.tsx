// Categories: Business rule (T-ui-sub-09 label disambiguation)
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { subTrack } from "@/test/subtitles"
import { SubtitleMenu } from "./SubtitleMenu"
import type { SubtitleUi } from "./useSubtitles"

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => vi.restoreAllMocks())

describe("SubtitleMenu labels", () => {
  it("T-ui-sub-09 [Business rule] same language+format → file name added; still identical (two SAMI classes of one file) → ordinals; ids are not parsed", async () => {
    // Arrange
    const tracks = [
      subTrack("a.smi#X", null, "smi", "a.smi"),
      subTrack("a.smi#Y", null, "smi", "a.smi"),
      subTrack("a.smi#Z", null, "smi", "a.smi"),
      subTrack("b.srt", null, "srt", "b.srt"),
      subTrack("c.ko.srt", "ko", "srt", "c.ko.srt"),
    ]
    const ui: SubtitleUi = { tracks, selectedId: null, effectiveId: null, select: vi.fn(), loading: false }
    render(<SubtitleMenu ui={ui} />)
    // Act
    await userEvent.setup().click(screen.getByRole("button", { name: "자막" }))
    // Assert
    const names = (await screen.findAllByRole("menuitemradio")).map((el) => el.textContent)
    expect(names).toEqual(["자막 끄기", "알 수 없음 (SMI) · a.smi", "알 수 없음 (SMI) · a.smi (2)", "알 수 없음 (SMI) · a.smi (3)", "알 수 없음 (SRT)", "한국어 (SRT)"])
  })
})
