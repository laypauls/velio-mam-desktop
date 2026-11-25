// Categories: Business rule (T-ui-ai-settings-01, T-ui-ai-settings-03) / Validation (T-ui-ai-settings-02)
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

async function openAiTab(user: User) {
  const dialog = await openSettingsPage(user, "외부 AI")
  await within(dialog).findByRole("region", { name: "외부 AI 전송 동의" })
  return dialog
}

describe("settings ai tab (P3)", () => {
  it("T-ui-ai-settings-01 [Business rule] after saving an api key the input is empty, the key is nowhere in the DOM, and only a status badge remains", async () => {
    // Arrange
    stubLayout()
    const secret = "sk-ant-SECRET-1234567890"
    const api = createMockApi({ count: 300 })
    const saveSpy = vi.spyOn(api, "setApiKey")
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openAiTab(user)
    const list = await within(dialog).findByRole("list", { name: "공급자 목록" })
    const input = within(list).getByLabelText("Claude API 키")
    expect(within(list).getAllByText("키: 없음")).toHaveLength(3)

    // Act
    await user.type(input, secret)
    await user.click(within(list).getByRole("button", { name: "Claude 키 저장" }))

    // Assert
    expect(await within(list).findByText("키: 저장됨")).toBeInTheDocument()
    expect(saveSpy).toHaveBeenCalledWith("claude", secret, true)
    expect(input).toHaveValue("")
    expect(document.body.innerHTML).not.toContain(secret)
    expect(JSON.stringify(await api.listAiProviders())).not.toContain("SECRET")

    // Act 2: 이번 실행만 저장(체크 해제) — 다른 공급자
    await user.click(within(dialog).getByRole("checkbox", { name: /이 PC 에 저장/ }))
    await user.type(within(list).getByLabelText("Gemini API 키"), secret)
    await user.click(within(list).getByRole("button", { name: "Gemini 키 저장" }))
    // Assert 2
    expect(await within(list).findByText("키: 이번 실행만")).toBeInTheDocument()
    expect(saveSpy).toHaveBeenLastCalledWith("gemini", secret, false)
    expect(document.body.innerHTML).not.toContain(secret)
  })

  it("T-ui-ai-settings-02 [Validation] out-of-range transfer values and schedule hours show inline errors and are not saved", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    const updateSpy = vi.spyOn(api, "updateAiSettings")
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openAiTab(user)
    const edit = async (label: string, value: string) => {
      const field = await within(dialog).findByLabelText(label)
      await user.clear(field)
      await user.type(field, value)
      await user.tab()
      return field
    }

    // Act + Assert: 이미지 긴 변 256~2048
    const edge = await edit("이미지 긴 변", "100")
    expect(await within(dialog).findByText("256 이상 2048 이하의 정수를 입력하세요.")).toBeInTheDocument()
    expect(edge).toHaveAttribute("aria-invalid", "true")
    // Act + Assert: 프레임 1~10
    const frames = await edit("동영상 프레임 수", "11")
    expect(await within(dialog).findByText("1 이상 10 이하의 정수를 입력하세요.")).toBeInTheDocument()
    expect(frames).toHaveAttribute("aria-invalid", "true")
    // Act + Assert: 예약 시각 0~23
    await user.click(within(dialog).getByRole("switch", { name: "예약 실행" }))
    await edit("시작 시각", "24")
    expect(await within(dialog).findByText("0 이상 23 이하의 정수를 입력하세요.")).toBeInTheDocument()
    expect(updateSpy).not.toHaveBeenCalledWith(expect.objectContaining({ transfer: expect.anything() }))
    expect(updateSpy).not.toHaveBeenCalledWith({ schedule: { startHour: 24 } })
    expect((await api.getAiSettings()).transfer.imageMaxEdge).toBe(1024)

    // Act + Assert: 올바른 값은 저장된다
    await edit("이미지 긴 변", "512")
    await waitFor(() => expect(updateSpy).toHaveBeenCalledWith({ transfer: { imageMaxEdge: 512 } }))
    expect((await api.getAiSettings()).transfer.imageMaxEdge).toBe(512)
    expect(edge).not.toHaveAttribute("aria-invalid")
  })

  it("T-ui-ai-settings-03 [Business rule] revoking consent disables and turns off the auto and background switches", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, ai: { consent: true } })
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openAiTab(user)
    const auto = await within(dialog).findByRole("switch", { name: "새 자산 자동 분석" })
    const background = within(dialog).getByRole("switch", { name: "미분석 자산 백그라운드 분석" })
    expect(auto).toBeEnabled()
    expect(background).toBeEnabled()
    await user.click(auto)
    await waitFor(() => expect(auto).toBeChecked())

    // Act
    await user.click(within(dialog).getByRole("button", { name: "동의 철회" }))

    // Assert
    await waitFor(() => expect(auto).toBeDisabled())
    expect(background).toBeDisabled()
    expect(auto).not.toBeChecked()
    const settings = await api.getAiSettings()
    expect(settings.consent.given).toBe(false)
    expect(settings.autoAnalyzeNew).toBe(false)
    expect(within(dialog).getByRole("button", { name: "동의하기" })).toBeInTheDocument()
  })
})
