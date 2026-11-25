// Categories: Happy (cards, timeline, band) / Boundary (wrap, all day, 0%, 60/65/75) / Validation (invalid hour keeps the status) / Business rule (a11y contract)
import { waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

type User = ReturnType<typeof userEvent.setup>

describe("settings visuals > preview cards (탐색·표시)", () => {
  it("T-ui-visual-01 [Business rule] paging and rail are radiogroups of cards: one radio checked, decorative previews are aria-hidden, selecting a card saves and moves aria-checked", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const update = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const page = await openSettingsPage(user, "탐색·표시")
    const paging = await within(page).findByRole("radiogroup", { name: "탐색 방식" })
    const rail = within(page).getByRole("radiogroup", { name: "목차 레일" })
    // Assert: 구성·기본값
    expect(within(paging).getAllByRole("radio").map((r) => r.getAttribute("aria-label"))).toEqual(["무한 스크롤", "페이지"])
    expect(within(rail).getAllByRole("radio").map((r) => r.getAttribute("aria-label"))).toEqual(["자동", "항상 표시", "끄기"])
    expect(within(paging).getByRole("radio", { name: "무한 스크롤" })).toHaveAttribute("aria-checked", "true")
    expect(within(rail).getByRole("radio", { name: "자동" })).toHaveAttribute("aria-checked", "true")
    for (const card of [...paging.querySelectorAll("label"), ...rail.querySelectorAll("label")]) {
      expect(card.firstElementChild).toHaveAttribute("aria-hidden", "true")
    }
    // Act
    await user.click(within(rail).getByRole("radio", { name: "끄기" }))
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ indexRail: "off" }))
    await waitFor(() => expect(within(rail).getByRole("radio", { name: "끄기" })).toHaveAttribute("aria-checked", "true"))
    expect(within(rail).getByRole("radio", { name: "자동" })).toHaveAttribute("aria-checked", "false")
  })

  it("T-ui-visual-02 [Happy] clicking the card text (not just the radio dot) selects it", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const update = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const page = await openSettingsPage(user, "탐색·표시")
    const paging = await within(page).findByRole("radiogroup", { name: "탐색 방식" })
    // Act: 카드 안의 설명 문구를 눌러도 라벨이 라디오를 고른다
    await user.click(within(paging).getByText("페이지는 번호로 이동합니다."))
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ galleryPaging: "paged" }))
  })
})

describe("settings visuals > schedule timeline (외부 AI)", () => {
  async function openSchedule(user: User) {
    stubLayout()
    const api = createMockApi({ count: 300 })
    renderApp(api)
    const dialog = await openSettingsPage(user, "외부 AI")
    await within(dialog).findByRole("region", { name: "외부 AI 전송 동의" })
    await user.click(within(dialog).getByRole("switch", { name: "예약 실행" }))
    const edit = async (label: string, value: string) => {
      const field = await within(dialog).findByLabelText(label)
      await user.clear(field)
      await user.type(field, value)
      await user.tab()
    }
    return { dialog, edit, api }
  }

  it("T-ui-visual-10 [Boundary] default 0 -> 0 reads as all day; 22 -> 6 reads as wrapping past midnight; 9 -> 17 as a same-day window", async () => {
    // Arrange
    const user = userEvent.setup()
    const { dialog, edit, api } = await openSchedule(user)
    // Assert: 시작 = 종료
    expect(await within(dialog).findByText("하루 종일 분석합니다.")).toBeInTheDocument()
    // Act + Assert: 자정 넘김
    await edit("시작 시각", "22")
    await edit("종료 시각", "6")
    expect(await within(dialog).findByText("매일 22시부터 다음 날 06시 전까지 분석합니다 (자정을 넘김).")).toBeInTheDocument()
    expect((await api.getAiSettings()).schedule).toMatchObject({ startHour: 22, endHour: 6 })
    // Act + Assert: 같은 날
    await edit("시작 시각", "9")
    await edit("종료 시각", "17")
    expect(await within(dialog).findByText("매일 09시부터 17시 전까지 분석합니다.")).toBeInTheDocument()
  })

  it("T-ui-visual-11 [Validation] an out-of-range hour is rejected inline and the timeline keeps the saved window; the bar itself is hidden from assistive tech", async () => {
    // Arrange
    const user = userEvent.setup()
    const { dialog, edit } = await openSchedule(user)
    await edit("시작 시각", "22")
    await edit("종료 시각", "6")
    await within(dialog).findByText(/자정을 넘김/)
    // Act
    await edit("시작 시각", "24")
    // Assert
    expect(await within(dialog).findByText("0 이상 23 이하의 정수를 입력하세요.")).toBeInTheDocument()
    const status = within(dialog).getByText(/매일 22시부터 다음 날 06시/)
    expect(status).toHaveAttribute("role", "status")
    expect(status.previousElementSibling).toHaveAttribute("aria-hidden", "true")
  })
})

describe("settings visuals > similarity band (로컬 AI·검색)", () => {
  async function openSimilar(min: number) {
    stubLayout()
    const api = createMockApi({ count: 100 })
    await api.updateSettings({ similarMinSimilarity: min })
    const user = userEvent.setup()
    renderApp(api)
    const main = await openSettingsPage(user, "로컬 AI·검색")
    return { user, api, section: within(await within(main).findByRole("region", { name: "유사 미디어" })) }
  }

  it("T-ui-visual-20 [Happy] the default 75% sits in the recommended zone and the band labels both zones", async () => {
    // Arrange / Act
    const { section } = await openSimilar(0.75)
    // Assert
    expect(await section.findByText("75% — 권장 구간입니다.")).toBeInTheDocument()
    expect(section.getByText("무관한 이미지 포함")).toBeInTheDocument()
    expect(section.getByText("권장")).toBeInTheDocument()
  })

  it("T-ui-visual-21 [Boundary] 0% reads as no limit", async () => {
    // Arrange / Act
    const { section } = await openSimilar(0)
    // Assert
    expect(await section.findByText("제한 없음 — 유사도와 관계없이 모두 표시합니다.")).toBeInTheDocument()
    expect(section.getByRole("slider", { name: "최소 유사도" })).toHaveAttribute("aria-valuenow", "0")
  })

  it("T-ui-visual-22 [Boundary] 60% is still the unrelated-images zone, 65% the middle zone", async () => {
    // Arrange / Act / Assert
    const sixty = await openSimilar(0.6)
    expect(await sixty.section.findByText("60% — 무관한 이미지도 포함될 수 있습니다.")).toBeInTheDocument()
  })

  it("T-ui-visual-23 [Boundary] 65% reads as the middle zone", async () => {
    // Arrange / Act
    const { section } = await openSimilar(0.65)
    // Assert
    expect(await section.findByText("65% — 보통입니다. 70% 이상을 권장합니다.")).toBeInTheDocument()
  })

  it("T-ui-visual-24 [Happy] moving the slider updates the band text and saves only the 0..1 value on commit", async () => {
    // Arrange
    const { user, api, section } = await openSimilar(0.75)
    const update = vi.spyOn(api, "updateSettings")
    const slider = await section.findByRole("slider", { name: "최소 유사도" })
    // Act: Home = 0%
    slider.focus()
    await user.keyboard("{Home}")
    // Assert
    expect(await section.findByText("제한 없음 — 유사도와 관계없이 모두 표시합니다.")).toBeInTheDocument()
    await waitFor(() => expect(update).toHaveBeenCalledWith({ similarMinSimilarity: 0 }))
  })
})
