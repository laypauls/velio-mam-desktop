// Categories: Happy (T-ui-filmstrip-store-01 enter/remember, 02 leave closes viewer) / Business rule (T-ui-filmstrip-store-03 closePreview, 04 unchanged outside filmstrip) / Boundary (T-ui-filmstrip-store-05 height clamp, 06 same mode no-op)
import { beforeEach, describe, expect, it } from "vitest"
import { resetAllStores } from "@/stores"
import { FILMSTRIP_DEFAULT_HEIGHT, useViewStore } from "./view"

const s = () => useViewStore.getState()

beforeEach(() => resetAllStores())

describe("view store — filmstrip mode", () => {
  it("T-ui-filmstrip-store-01 [Happy] entering remembers the previous grid/list mode", () => {
    s().setViewMode("list")
    s().setViewMode("filmstrip")
    expect(s().viewMode).toBe("filmstrip")
    expect(s().prevViewMode).toBe("list")
  })

  it("T-ui-filmstrip-store-02 [Business rule] leaving via the grid/list buttons closes the viewer pane and fullscreen (D2)", () => {
    s().setViewMode("filmstrip")
    s().openPreview(7, 1200)
    useViewStore.setState({ fullscreen: true })
    s().setViewMode("list")
    expect(s().viewMode).toBe("list")
    expect(s().prevViewMode).toBe("list")
    expect([s().previewId, s().seekMs, s().fullscreen]).toEqual([null, null, false])
  })

  it("T-ui-filmstrip-store-03 [Business rule] closePreview inside filmstrip returns to the previous mode (Esc, Q, ✕ share this path)", () => {
    s().setViewMode("list")
    s().setViewMode("filmstrip")
    s().openPreview(3)
    s().closePreview()
    expect(s().viewMode).toBe("list")
    expect(s().previewId).toBeNull()
  })

  it("T-ui-filmstrip-store-04 [Business rule] closePreview outside filmstrip keeps the mode (regression: split pane close)", () => {
    s().openPreview(3)
    s().closePreview()
    expect(s().viewMode).toBe("grid")
    expect(s().previewId).toBeNull()
  })

  it("T-ui-filmstrip-store-05 [Boundary] filmstripHeight is clamped 72..160, rounds, and only bumps layoutRev on a real change", () => {
    expect(s().filmstripHeight).toBe(FILMSTRIP_DEFAULT_HEIGHT)
    s().setFilmstripHeight(10)
    expect(s().filmstripHeight).toBe(72)
    s().setFilmstripHeight(999)
    expect(s().filmstripHeight).toBe(160)
    s().setFilmstripHeight(120.6)
    expect(s().filmstripHeight).toBe(121)
    const rev = s().layoutRev
    s().setFilmstripHeight(121)
    expect(s().layoutRev).toBe(rev)
    s().setFilmstripHeight(Number.NaN)
    expect(s().filmstripHeight).toBe(FILMSTRIP_DEFAULT_HEIGHT)
  })

  it("T-ui-filmstrip-store-06 [Boundary] selecting the current mode again changes nothing; hydrateLayout clamps the stored height and is ignored after a user change", () => {
    s().setViewMode("filmstrip")
    s().openPreview(2)
    s().setViewMode("filmstrip")
    expect(s().previewId).toBe(2)
    s().hydrateLayout({ detailOpen: true, sidebarOpen: true, viewerWidth: 520, detailTab: "info", filmstripHeight: 500 })
    expect(s().filmstripHeight).toBe(160)
    s().setFilmstripHeight(80)
    s().hydrateLayout({ detailOpen: true, sidebarOpen: true, viewerWidth: 520, detailTab: "info", filmstripHeight: 140 })
    expect(s().filmstripHeight).toBe(80)
  })
})
