// Categories: Happy (T-ui-capcard-01, 03) / Boundary (T-ui-capcard-02) / Business rule (T-ui-capcard-02~04)
import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"
import "@/i18n"
import { formatDateCompact } from "@/lib/format"
import type { AssetSummary, CapturedAtFallback, GalleryCardDisplay } from "@/lib/ipc-contract"
import { AssetCardInfo } from "./AssetCardInfo"
import { DEFAULT_CARD_DISPLAY } from "./cardDisplay"

const CAPTURED = Date.UTC(2026, 7, 2, 12)
const MODIFIED = Date.UTC(2026, 9, 20, 12)

const asset = (capturedAt: number | null): AssetSummary => ({
  assetId: 1,
  fileName: "a.jpg",
  mediaType: "image",
  availability: "online",
  width: 100,
  height: 50,
  durationMs: null,
  fileSize: 1000,
  capturedAt,
  modifiedAt: MODIFIED,
  favorite: false,
  rating: 0,
  thumbState: "pending",
  aiState: "none",
  title: null,
  revision: 1,
})

function renderInfo(a: AssetSummary, mode: CapturedAtFallback, display: GalleryCardDisplay = DEFAULT_CARD_DISPLAY) {
  return render(<AssetCardInfo asset={a} display={display} cellWidth={240} capturedAtFallback={mode} />)
}

const has = (ms: number) => screen.queryByText((t) => t.includes(formatDateCompact(ms)))

describe("card date follows capturedAtFallback", () => {
  it("T-ui-capcard-01 [Happy] with a captured date, both modes show the captured date", () => {
    renderInfo(asset(CAPTURED), "none")
    expect(has(CAPTURED)).not.toBeNull()
    expect(has(MODIFIED)).toBeNull()
  })

  it("T-ui-capcard-02 [Business rule] no captured date: 'modified' shows the modified date, 'none' shows no date", () => {
    const { unmount } = renderInfo(asset(null), "modified")
    expect(has(MODIFIED)).not.toBeNull()
    unmount()
    renderInfo(asset(null), "none")
    expect(has(MODIFIED)).toBeNull()
  })

  it("T-ui-capcard-03 [Boundary] dateBy 'modified' ignores the setting", () => {
    renderInfo(asset(CAPTURED), "none", { ...DEFAULT_CARD_DISPLAY, dateBy: "modified" })
    expect(has(MODIFIED)).not.toBeNull()
  })

  it("T-ui-capcard-04 [Boundary] default prop keeps the previous behavior (modified fallback)", () => {
    render(<AssetCardInfo asset={asset(null)} display={DEFAULT_CARD_DISPLAY} cellWidth={240} />)
    expect(has(MODIFIED)).not.toBeNull()
  })
})
