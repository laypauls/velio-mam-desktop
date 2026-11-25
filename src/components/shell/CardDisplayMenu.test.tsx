// Categories: Happy (T-ui-cdm-01, 02, 05, 13, 14) / Business rule (T-ui-cdm-03, 04, 06, 12, 16) / External (T-ui-cdm-07, 08) / Validation (T-ui-cdm-09, 17) / Boundary (T-ui-cdm-15)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { DEFAULT_CARD_DISPLAY, PRESETS } from "@/components/gallery/cardDisplay"
import { createMockApi } from "@/lib/api/mock"
import type { GalleryCardDisplay } from "@/lib/ipc-contract"
import { useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const RESOLUTION = /\d+×\d+/

async function setup(api = createMockApi({ count: 200 })) {
  stubLayout({ width: 1000, height: 600 })
  const user = userEvent.setup()
  renderApp(api)
  await screen.findAllByRole("option")
  const openMenu = async () => {
    await user.click(screen.getByRole("button", { name: "표시 옵션" }))
    return screen.findByRole("menu")
  }
  const resolutionCount = () => screen.queryAllByText(RESOLUTION).length
  return { api, user, openMenu, resolutionCount }
}

describe("CardDisplayMenu", () => {
  it("T-ui-cdm-01 [Happy] toggling an item keeps the menu open, updates the cards and switches the header to 'custom'", async () => {
    // Arrange
    const { user, openMenu, resolutionCount } = await setup()
    expect(resolutionCount()).toBeGreaterThan(0)
    const menu = await openMenu()
    expect(within(menu).getByText("자세히", { selector: "span" })).toBeInTheDocument()
    // Act
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: /^해상도/ }))
    // Assert
    expect(screen.getByRole("menu")).toBeInTheDocument()
    expect(resolutionCount()).toBe(0)
    expect(within(menu).getByText("사용자 지정")).toBeInTheDocument()
    expect(within(menu).getByRole("menuitemcheckbox", { name: /^해상도/ })).toHaveAttribute("aria-checked", "false")
    // 다시 켜면 자세히로 돌아온다
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: /^해상도/ }))
    expect(resolutionCount()).toBeGreaterThan(0)
    expect(within(menu).getByText("자세히", { selector: "span" })).toBeInTheDocument()
  })

  it("T-ui-cdm-02 [Happy] selecting a preset applies all six items at once and keeps the menu open", async () => {
    // Arrange
    const { user, openMenu, resolutionCount } = await setup()
    const menu = await openMenu()
    // Act
    await user.click(within(menu).getByRole("menuitemradio", { name: "썸네일만" }))
    // Assert
    expect(screen.getByRole("menu")).toBeInTheDocument()
    expect(useViewStore.getState().cardDisplay.info).toEqual(PRESETS.off)
    expect(within(menu).getByRole("menuitemradio", { name: "썸네일만" })).toHaveAttribute("aria-checked", "true")
    expect(resolutionCount()).toBe(0)
    // Act: 간단
    await user.click(within(menu).getByRole("menuitemradio", { name: "간단" }))
    expect(useViewStore.getState().cardDisplay.info).toEqual(PRESETS.compact)
  })

  it("T-ui-cdm-03 [Business rule] 'reset to defaults' restores only the card display (card size is kept) and keeps the menu open", async () => {
    // Arrange
    const { user, openMenu } = await setup()
    act(() => {
      useViewStore.getState().setCardSize("xl")
      useViewStore.getState().setCardDisplay({ check: "always", favorite: false, dateBy: "modified" })
      useViewStore.getState().applyCardPreset("off")
    })
    const menu = await openMenu()
    // Act
    await user.click(within(menu).getByRole("menuitem", { name: "기본값으로" }))
    // Assert
    expect(useViewStore.getState().cardDisplay).toEqual({ ...DEFAULT_CARD_DISPLAY, cardSize: "xl" })
    expect(screen.getByRole("menu")).toBeInTheDocument()
  })

  it("T-ui-cdm-04 [Business rule] the status item is checked and disabled with a 'cannot turn off' hint", async () => {
    // Arrange + Act
    const { openMenu } = await setup()
    const menu = await openMenu()
    // Assert
    const status = within(menu).getByRole("menuitemcheckbox", { name: /오프라인·파일 없음/ })
    expect(status).toHaveAttribute("aria-checked", "true")
    expect(status).toHaveAttribute("aria-disabled", "true")
    expect(status).toHaveTextContent("끌 수 없음")
  })

  it("T-ui-cdm-05 [Happy] check mode radio, date basis radio and overlay flags write through to the store", async () => {
    // Arrange
    const { user, openMenu } = await setup()
    const menu = await openMenu()
    // Act
    await user.click(within(menu).getByRole("menuitemradio", { name: "항상" }))
    await user.click(within(menu).getByRole("menuitemradio", { name: "수정일" }))
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: "즐겨찾기" }))
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: "재생 길이" }))
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: /^빠른 동작/ }))
    // Assert
    expect(useViewStore.getState().cardDisplay).toEqual<GalleryCardDisplay>({
      ...DEFAULT_CARD_DISPLAY,
      check: "always",
      dateBy: "modified",
      favorite: false,
      duration: false,
      quickActions: false,
    })
  })

  it("T-ui-cdm-06 [Business rule] the trigger is disabled in list view and enabled again in grid view", async () => {
    // Arrange
    await setup()
    const trigger = screen.getByRole("button", { name: "표시 옵션" })
    expect(trigger).toBeEnabled()
    // Act + Assert
    act(() => useViewStore.getState().setViewMode("list"))
    expect(trigger).toBeDisabled()
    act(() => useViewStore.getState().setViewMode("grid"))
    expect(trigger).toBeEnabled()
  })
})

describe("card display persistence", () => {
  it("T-ui-cdm-07 [Happy] changes are saved once, debounced, as the whole object via updateSettings", async () => {
    // Arrange
    const { api, user, openMenu } = await setup()
    const update = vi.spyOn(api, "updateSettings")
    const menu = await openMenu()
    // Act: 연달아 두 번 바꾼다
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: /^해상도/ }))
    await user.click(within(menu).getByRole("menuitemradio", { name: "숨김" }))
    // Assert
    await waitFor(() => expect(update).toHaveBeenCalled())
    expect(update).toHaveBeenCalledTimes(1)
    expect(update).toHaveBeenCalledWith({
      galleryCardDisplay: { ...DEFAULT_CARD_DISPLAY, info: { ...DEFAULT_CARD_DISPLAY.info, resolution: false }, check: "hidden" },
    })
    expect((await api.getSettings()).galleryCardDisplay.check).toBe("hidden")
  })

  it("T-ui-cdm-08 [External] a failed save shows an error toast and keeps the local value", async () => {
    // Arrange
    const { api, user, openMenu, resolutionCount } = await setup()
    vi.spyOn(api, "updateSettings").mockRejectedValue({ code: "io", message: "disk" })
    const menu = await openMenu()
    // Act
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: /^해상도/ }))
    // Assert
    expect(await screen.findByText("파일 입출력 중 오류가 발생했습니다.")).toBeInTheDocument()
    expect(useViewStore.getState().cardDisplay.info.resolution).toBe(false)
    expect(resolutionCount()).toBe(0)
  })

  it("T-ui-cdm-09 [Validation] stored values hydrate the store at startup; broken stored values fall back field by field", async () => {
    // Arrange: 저장값 = 간단 + 수정일 기준 + 알 수 없는 필드 + 깨진 check
    const api = createMockApi({ count: 200 })
    const stored = { ...(await api.getSettings()), galleryCardDisplay: { info: PRESETS.compact, dateBy: "modified", check: 5, extra: 1 } }
    vi.spyOn(api, "getSettings").mockResolvedValue(stored as never)
    // Act
    const { openMenu, resolutionCount } = await setup(api)
    // Assert
    await waitFor(() => expect(useViewStore.getState().cardDisplay.info).toEqual(PRESETS.compact))
    expect(useViewStore.getState().cardDisplay).toEqual({ ...DEFAULT_CARD_DISPLAY, info: PRESETS.compact, dateBy: "modified" })
    expect(resolutionCount()).toBe(0)
    const menu = await openMenu()
    expect(within(menu).getByRole("menuitemradio", { name: "간단" })).toHaveAttribute("aria-checked", "true")
    // 저장값을 채우는 것만으로는 다시 저장하지 않는다
    expect(useViewStore.getState().cardDisplayRev).toBe(0)
  })

  it("T-ui-cdm-10 [Concurrency] a change made before settings finish loading wins over the stored value and is saved", async () => {
    // Arrange: 느린 getSettings
    const api = createMockApi({ count: 200 })
    const real = await api.getSettings()
    let release: (v: typeof real) => void = () => undefined
    vi.spyOn(api, "getSettings").mockReturnValue(new Promise((r) => (release = r)))
    const update = vi.spyOn(api, "updateSettings")
    stubLayout({ width: 1000, height: 600 })
    renderApp(api)
    // Act: 사용자가 먼저 바꾼 뒤 저장값 도착
    act(() => useViewStore.getState().setCardDisplay({ check: "always" }))
    await act(async () => release({ ...real, galleryCardDisplay: { ...DEFAULT_CARD_DISPLAY, check: "hidden" } }))
    // Assert
    expect(useViewStore.getState().cardDisplay.check).toBe("always")
    await waitFor(() => expect(update).toHaveBeenCalledWith({ galleryCardDisplay: expect.objectContaining({ check: "always" }) }))
  })
})

describe("segmented rows keyboard", () => {
  it("T-ui-cdm-11 [Happy] arrow keys reach a segment, Enter and Space select it, and the menu stays open; each group has an accessible name", async () => {
    // Arrange
    const { user } = await setup()
    screen.getByRole("button", { name: "표시 옵션" }).focus()
    // Act: 키보드로 열면 첫 항목(카드 크기 XS)에 포커스, 아래로 다섯 칸(S·M·L·XL·썸네일만) → 썸네일만
    await user.keyboard("{Enter}")
    const menu = await screen.findByRole("menu")
    expect(within(menu).getByRole("menuitemradio", { name: "XS · 120px" })).toHaveFocus()
    await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}{ArrowDown}{ArrowDown}")
    expect(within(menu).getByRole("menuitemradio", { name: "썸네일만" })).toHaveFocus()
    await user.keyboard("{ArrowDown}")
    expect(within(menu).getByRole("menuitemradio", { name: "간단" })).toHaveFocus()
    await user.keyboard("{Enter}")
    // Assert
    expect(useViewStore.getState().cardDisplay.info).toEqual(PRESETS.compact)
    expect(screen.getByRole("menu")).toBeInTheDocument()
    // Act: 다음 세그먼트 + Space
    await user.keyboard("{ArrowDown}")
    await user.keyboard(" ")
    expect(useViewStore.getState().cardDisplay.info).toEqual(PRESETS.detailed)
    expect(screen.getByRole("menu")).toBeInTheDocument()
    for (const name of ["미리 설정", "날짜 기준", "선택 체크"]) {
      expect(within(menu).getByRole("group", { name })).toBeInTheDocument()
    }
  })
})

describe("card size row", () => {
  const cellWidths = () => // 메뉴가 열리면 바깥은 aria-hidden 이라 role 질의가 안 된다 — data 속성으로 찾는다
    [...document.querySelectorAll<HTMLElement>("[data-asset-id]")].map((o) => parseFloat(o.parentElement?.style.width ?? "NaN"))
  const SIZE_NAMES = ["XS · 120px", "S · 150px", "M · 180px", "L · 240px", "XL · 300px"]

  it("T-ui-cdm-12 [Business rule] the size row is a 5-segment radio group (no slider) with px in each accessible name; M is selected by default", async () => {
    // Arrange + Act
    const { openMenu } = await setup()
    expect(screen.queryByRole("slider", { name: "카드 크기" })).not.toBeInTheDocument() // 목차 레일(slider)이 같은 화면에 있다
    const menu = await openMenu()
    // Assert
    const group = within(menu).getByRole("group", { name: "카드 크기" })
    const radios = within(group).getAllByRole("menuitemradio")
    expect(radios.map((r) => r.getAttribute("aria-label"))).toEqual(SIZE_NAMES)
    expect(radios.map((r) => r.textContent)).toEqual(["XS", "S", "M", "L", "XL"])
    expect(radios.map((r) => r.getAttribute("aria-checked"))).toEqual(["false", "false", "true", "false", "false"])
  })

  it("T-ui-cdm-13 [Happy] selecting a segment updates the store and the card width, and the menu stays open", async () => {
    // Arrange
    const { user, openMenu } = await setup()
    const menu = await openMenu()
    const before = cellWidths()[0] // 1000px 창, M: 5열 188.8
    expect(before).toBeCloseTo(188.8, 6)
    // Act
    await user.click(within(menu).getByRole("menuitemradio", { name: "XL · 300px" }))
    // Assert: inner 976 -> 3열 (976-16)/3 = 320
    expect(useViewStore.getState().cardDisplay.cardSize).toBe("xl")
    expect(cellWidths()[0]).toBeCloseTo(320, 6)
    expect(within(menu).getByRole("menuitemradio", { name: "XL · 300px" })).toHaveAttribute("aria-checked", "true")
    expect(screen.getByRole("menu")).toBeInTheDocument()
  })

  it("T-ui-cdm-14 [Happy] keyboard: the menu opens on the first size segment, ArrowDown moves through the segments, Enter selects, the menu stays open", async () => {
    // Arrange
    const { user } = await setup()
    screen.getByRole("button", { name: "표시 옵션" }).focus()
    await user.keyboard("{Enter}")
    const menu = await screen.findByRole("menu")
    expect(within(menu).getByRole("menuitemradio", { name: "XS · 120px" })).toHaveFocus()
    // Act: XS -> S -> M -> L
    await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}")
    expect(within(menu).getByRole("menuitemradio", { name: "L · 240px" })).toHaveFocus()
    await user.keyboard("{Enter}")
    // Assert
    expect(useViewStore.getState().cardDisplay.cardSize).toBe("l")
    expect(screen.getByRole("menu")).toBeInTheDocument()
    expect(within(menu).getByRole("menuitemradio", { name: "L · 240px" })).toHaveFocus()
  })

  it("T-ui-cdm-15 [Boundary] every size produces a filled row: the first row of cards ends exactly at the container inner width", async () => {
    // Arrange
    const { user, openMenu } = await setup()
    const menu = await openMenu()
    for (const [i, name] of SIZE_NAMES.entries()) {
      // Act
      await user.click(within(menu).getByRole("menuitemradio", { name }))
      // Assert: 같은 폭 · 첫 행 합 = inner(1000-24)
      const widths = cellWidths().slice(0, Math.round(1008 / ([120, 150, 180, 240, 300][i] + 8)))
      const sum = widths.reduce((a, w) => a + w, 0) + 8 * (widths.length - 1)
      expect(sum).toBeCloseTo(976, 6)
    }
  })

  it("T-ui-cdm-16 [Business rule] the size is persisted with the display options (debounced) and 'reset to defaults' keeps it", async () => {
    // Arrange
    const { api, user, openMenu } = await setup()
    const update = vi.spyOn(api, "updateSettings")
    const menu = await openMenu()
    // Act
    await user.click(within(menu).getByRole("menuitemradio", { name: "L · 240px" }))
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1))
    expect(update).toHaveBeenLastCalledWith({ galleryCardDisplay: { ...DEFAULT_CARD_DISPLAY, cardSize: "l" } })
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: /^해상도/ }))
    await user.click(within(menu).getByRole("menuitem", { name: "기본값으로" }))
    // Assert: 표시 옵션은 기본값, 크기는 유지, 저장에도 반영
    expect(useViewStore.getState().cardDisplay).toEqual({ ...DEFAULT_CARD_DISPLAY, cardSize: "l" })
    await waitFor(() => expect(update).toHaveBeenCalledTimes(2))
    expect(update).toHaveBeenLastCalledWith({ galleryCardDisplay: { ...DEFAULT_CARD_DISPLAY, cardSize: "l" } })
    expect((await api.getSettings()).galleryCardDisplay.cardSize).toBe("l")
  })

  it("T-ui-cdm-17 [Validation] a stored size hydrates at startup without re-saving", async () => {
    // Arrange
    const api = createMockApi({ count: 200 })
    const base = await api.getSettings()
    vi.spyOn(api, "getSettings").mockResolvedValue({ ...base, galleryCardDisplay: { ...base.galleryCardDisplay, cardSize: "xs" } })
    // Act
    await setup(api)
    // Assert
    await waitFor(() => expect(useViewStore.getState().cardDisplay.cardSize).toBe("xs"))
    expect(useViewStore.getState().cardDisplayRev).toBe(0)
  })
})
