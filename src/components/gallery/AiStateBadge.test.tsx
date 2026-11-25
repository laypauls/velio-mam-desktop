// Categories: Happy (T-ui-aistate-01, 02) / Boundary (T-ui-aistate-03) / Business rule (T-ui-aistate-04, 05)
import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"
import "@/i18n"
import type { AssetSummary } from "@/lib/ipc-contract"
import { AssetCardInfo } from "./AssetCardInfo"
import { DEFAULT_CARD_DISPLAY } from "./cardDisplay"

const base: AssetSummary = {
  assetId: 3,
  fileName: "clip.mp4",
  mediaType: "video",
  availability: "online",
  width: 640,
  height: 360,
  durationMs: 3000,
  fileSize: 2_000_000,
  capturedAt: null,
  modifiedAt: Date.UTC(2026, 9, 20, 12),
  favorite: false,
  rating: 0,
  thumbState: "done",
  aiState: "none",
  title: null,
  revision: 1,
}

function renderInfo(over: Partial<AssetSummary>, typeIcon = true) {
  return render(
    <AssetCardInfo
      asset={{ ...base, ...over }}
      display={{ ...DEFAULT_CARD_DISPLAY, info: { ...DEFAULT_CARD_DISPLAY.info, typeIcon } }}
      cellWidth={240}
    />,
  )
}

describe("card AI state badge", () => {
  it("T-ui-aistate-01 [Happy] a card with an AI result shows the 'done' badge with tooltip and screen-reader text", () => {
    renderInfo({ aiState: "done" })
    expect(screen.getByText("AI 분석 완료")).toBeInTheDocument()
    expect(screen.getByTitle("AI 분석 완료")).toBeInTheDocument()
    expect(screen.queryByText("AI 분석 실패")).not.toBeInTheDocument()
  })

  it("T-ui-aistate-02 [Happy] a failed card shows the 'failed' marker, not the done badge", () => {
    renderInfo({ aiState: "failed" })
    expect(screen.getByText("AI 분석 실패")).toBeInTheDocument()
    expect(screen.getByTitle("AI 분석 실패")).toBeInTheDocument()
    expect(screen.queryByText("AI 분석 완료")).not.toBeInTheDocument()
  })

  it("T-ui-aistate-03 [Boundary] an unanalyzed card shows no AI marker", () => {
    renderInfo({ aiState: "none" })
    expect(screen.queryByText(/AI 분석/)).not.toBeInTheDocument()
    expect(screen.queryByTitle(/AI 분석/)).not.toBeInTheDocument()
  })

  it("T-ui-aistate-04 [Business rule] turning the type icon off hides the badge too, whatever the state", () => {
    renderInfo({ aiState: "done" }, false)
    expect(screen.queryByText(/AI 분석/)).not.toBeInTheDocument()
    renderInfo({ aiState: "failed" }, false)
    expect(screen.queryByText(/AI 분석/)).not.toBeInTheDocument()
  })

  it("T-ui-aistate-05 [Business rule] the name row content is the same with and without the badge", () => {
    const { container: a } = renderInfo({ aiState: "none" })
    const none = a.querySelector("[title='clip.mp4']")?.textContent
    const { container: b } = renderInfo({ aiState: "done" })
    expect(b.querySelector("[title='clip.mp4']")?.textContent).toBe(none)
    expect(b.querySelector("[title='clip.mp4']")?.textContent).toBe("clip")
  })
})
