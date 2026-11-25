// Categories: Happy (T-ui-settings-ranges-01, 02) / Business rule (T-ui-settings-ranges-03 conflict warning) / Validation (T-ui-settings-ranges-04 hide-shortcut capture, 05 no empty select value)
import { waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { SettingsPatch } from "@/lib/ipc-contract"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup(settings: SettingsPatch = {}) {
  stubLayout()
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  const api = createMockApi({ count: 50 })
  if (Object.keys(settings).length > 0) await api.updateSettings(settings)
  const update = vi.spyOn(api, "updateSettings")
  const user = userEvent.setup()
  renderApp(api)
  const page = await openSettingsPage(user, "플레이어")
  await within(page).findByRole("region", { name: "플레이어" })
  return { api, update, user, page }
}

describe("settings > player > open autoplay and range keys", () => {
  it("T-ui-settings-ranges-01 [Happy] open-autoplay is a radio group defaulting to 'open paused'; choosing 자동 재생 saves mediaOpenAutoplay and the note says collection playback always autoplays", async () => {
    // Arrange
    const { update, user, page } = await setup()
    const group = within(page).getByRole("radiogroup", { name: "동영상·음성 열 때" })
    const paused = within(group).getByRole("radio", { name: "정지 상태로 열기" })
    const auto = within(group).getByRole("radio", { name: "자동 재생" })
    // Assert: 기본
    expect(paused).toBeChecked()
    expect(auto).not.toBeChecked()
    expect(within(page).getByText("컬렉션 연속 재생은 이 설정과 관계없이 항상 자동 재생합니다.")).toBeInTheDocument()
    // Act
    await user.click(auto)
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ mediaOpenAutoplay: true }))
    await waitFor(() => expect(auto).toBeChecked())
    // Act: 되돌리기
    await user.click(within(page).getByRole("radio", { name: "정지 상태로 열기" }))
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ mediaOpenAutoplay: false }))
  })

  it("T-ui-settings-ranges-02 [Happy] the range-keys preset is a radiogroup defaulting to brackets with a keycap table; choosing Premiere saves playRangeKeys and the table switches at once", async () => {
    // Arrange
    const { update, user, page } = await setup()
    const group = within(page).getByRole("radiogroup", { name: "재생 구간 단축키" })
    const brackets = within(group).getByRole("radio", { name: "괄호 [ ] (기본)" })
    const premiere = within(group).getByRole("radio", { name: "Premiere I O" })
    const guide = within(page).getByLabelText("선택한 프리셋의 키")
    // Assert: 기본(괄호)
    expect(brackets).toBeChecked()
    expect(premiere).not.toBeChecked()
    expect(within(guide).getByText("시작(In) 표시").nextElementSibling).toHaveTextContent("[")
    expect(within(guide).getByText("이전 구간 시작으로 이동").nextElementSibling).toHaveTextContent("Shift+[")
    expect(within(guide).getByText("이전 구간 시작으로 이동").nextElementSibling?.querySelectorAll("kbd")).toHaveLength(2)
    expect(within(guide).getByText("선택한 구간 삭제").nextElementSibling).toHaveTextContent("Delete")
    // Act
    await user.click(premiere)
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ playRangeKeys: "premiere" }))
    await waitFor(() => expect(premiere).toBeChecked())
    expect(within(guide).getByText("시작(In) 표시").nextElementSibling).toHaveTextContent("I")
    expect(within(guide).getByText("현재 위치의 구간 선택").nextElementSibling).toHaveTextContent("X")
    expect(within(guide).getByText("대기 시작 지우기").nextElementSibling).toHaveTextContent("Ctrl+Shift+I")
  })

  it("T-ui-settings-ranges-03 [Business rule] an inline warning appears on the preset group only while the selected preset clashes with the current hide shortcut", async () => {
    // Arrange: 숨기기 키 "I" — Premiere 프리셋의 In 키
    const { user, page } = await setup({ hideShortcut: "I" })
    const group = within(page).getByRole("radiogroup", { name: "재생 구간 단축키" })
    // Assert 1: 괄호 프리셋은 겹치지 않는다
    expect(page).not.toHaveTextContent("숨기기 단축키와")
    expect(group).not.toHaveAttribute("aria-invalid")
    // Act: Premiere 로 변경
    await user.click(within(group).getByRole("radio", { name: "Premiere I O" }))
    // Assert 2
    const warning = await within(page).findByText("숨기기 단축키와 I 키가 겹칩니다. 겹치는 동안은 숨기기 단축키가 먼저 동작합니다.")
    expect(group).toHaveAttribute("aria-invalid", "true")
    expect(group).toHaveAttribute("aria-describedby", warning.id)
  })

  it("T-ui-settings-ranges-04 [Validation] the hide-shortcut recorder refuses a key the selected preset uses (O under premiere) with an inline alert, and accepts it under brackets", async () => {
    // Arrange: Premiere 프리셋
    const { update, user, page } = await setup({ playRangeKeys: "premiere" })
    update.mockClear()
    const recorder = within(page).getByRole("button", { name: "재생 정지 후 숨기기 단축키" })
    // Act 1
    await user.click(recorder)
    await user.keyboard("o")
    // Assert 1
    expect(await within(page).findByRole("alert")).toHaveTextContent("O 키는 다른 기능에 쓰이고 있습니다")
    expect(update).not.toHaveBeenCalled()
    expect(recorder).toHaveAttribute("aria-invalid", "true")
  })

  it("T-ui-settings-ranges-04b [Validation] under the brackets preset the same key O is a valid hide shortcut", async () => {
    // Arrange
    const { update, user, page } = await setup()
    const recorder = within(page).getByRole("button", { name: "재생 정지 후 숨기기 단축키" })
    // Act
    await user.click(recorder)
    await user.keyboard("o")
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ hideShortcut: "O" }))
  })

  it("T-ui-settings-ranges-05 [Validation] the preset group offers exactly the two presets, each card showing the In/Out keycaps; selecting the already-selected card does not save", async () => {
    // Arrange
    const { update, user, page } = await setup()
    const group = within(page).getByRole("radiogroup", { name: "재생 구간 단축키" })
    // Assert
    const radios = within(group).getAllByRole("radio")
    expect(radios.map((r) => r.getAttribute("aria-label"))).toEqual(["괄호  [ ]  (기본)", "Premiere  I O"])
    const cardKeys = (name: string) => [...(within(group).getByRole("radio", { name }).closest("label")?.querySelectorAll("kbd") ?? [])].map((k) => k.textContent)
    expect(cardKeys("괄호 [ ] (기본)")).toEqual(["[", "]"])
    expect(cardKeys("Premiere I O")).toEqual(["I", "O"])
    // Act: 이미 선택된 카드
    await user.click(radios[0])
    // Assert
    expect(update).not.toHaveBeenCalled()
  })
})
