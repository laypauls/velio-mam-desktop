// Categories: Happy (T-ui-simset-01,02) / Validation (T-ui-simset-03,04) / External (T-ui-simset-05)
import { waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { cmdError } from "@/lib/api/mock-search"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

type User = ReturnType<typeof userEvent.setup>

async function openSection(user: User) {
  const main = await openSettingsPage(user, "로컬 AI·검색")
  return within(await within(main).findByRole("region", { name: "유사 미디어" }))
}

describe("settings ai > similar media", () => {
  it("T-ui-simset-01 [Happy] shows the saved defaults and the hint about unrelated images", async () => {
    // Arrange / Act
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 100 }))
    const section = await openSection(user)
    // Assert
    expect(await section.findByLabelText("최대 표시 개수")).toHaveValue(24)
    expect(section.getByRole("slider", { name: "최소 유사도" })).toHaveAttribute("aria-valuenow", "75")
    expect(section.getByText("75%")).toBeInTheDocument()
    expect(section.getByText(/50~60%/)).toBeInTheDocument()
  })

  it("T-ui-simset-02 [Happy] committing the max count (blur) saves it; the slider saves on commit with a 0..1 value", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 100 })
    const update = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const section = await openSection(user)
    // Act 1: 숫자 입력
    const max = await section.findByLabelText("최대 표시 개수")
    await user.clear(max)
    await user.type(max, "40")
    await user.tab()
    // Assert 1
    await waitFor(() => expect(update).toHaveBeenCalledWith({ similarMaxResults: 40 }))
    // Act 2: 슬라이더 (75 → 80, 키보드 한 칸 = 커밋 1회)
    const slider = section.getByRole("slider", { name: "최소 유사도" })
    slider.focus()
    await user.keyboard("{ArrowRight}")
    // Assert 2
    await waitFor(() => expect(update).toHaveBeenCalledWith({ similarMinSimilarity: 0.8 }))
    expect((await api.getSettings()).similarMinSimilarity).toBe(0.8)
    expect(update).toHaveBeenCalledTimes(2)
  })

  it("T-ui-simset-03 [Validation] out-of-range max shows an inline error and is not saved", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 100 })
    const update = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const section = await openSection(user)
    const max = await section.findByLabelText("최대 표시 개수")
    for (const bad of ["5", "201", "12.5"]) {
      // Act
      await user.clear(max)
      await user.type(max, bad)
      await user.tab()
      // Assert
      expect(await section.findByText("6 이상 200 이하의 정수를 입력하세요.")).toBeInTheDocument()
      expect(max).toHaveAttribute("aria-invalid", "true")
    }
    expect(update).not.toHaveBeenCalled()
  })

  it("T-ui-simset-04 [Validation] slider is bounded to 0..99 in 5% steps", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 100 }))
    const section = await openSection(user)
    // Assert
    const slider = section.getByRole("slider", { name: "최소 유사도" })
    expect(slider).toHaveAttribute("aria-valuemin", "0")
    expect(slider).toHaveAttribute("aria-valuemax", "99")
  })

  it("T-ui-simset-05 [External] a rejected slider save shows an inline alert and snaps back to the saved value", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 100 })
    vi.spyOn(api, "updateSettings").mockRejectedValue(cmdError("invalid_input", "similarMinSimilarity must be 0..0.99"))
    const user = userEvent.setup()
    renderApp(api)
    const section = await openSection(user)
    const slider = section.getByRole("slider", { name: "최소 유사도" })
    // Act
    slider.focus()
    await user.keyboard("{ArrowRight}")
    // Assert
    expect(await section.findByRole("alert")).toBeInTheDocument()
    await waitFor(() => expect(slider).toHaveAttribute("aria-valuenow", "75"))
  })
})
