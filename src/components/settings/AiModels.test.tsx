// Categories: Business rule (T-ui-ai-models-01 - e5 gates semantic search, CLIP gates similar media)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { useSelectionStore } from "@/stores"
import { findAsset } from "@/test/ai-helpers"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

type User = ReturnType<typeof userEvent.setup>

async function openSimilar(user: User, assetId: number) {
  act(() => useSelectionStore.getState().selectOnly(assetId))
  const panel = screen.getByRole("complementary", { name: "상세 정보 패널" })
  await user.click(await within(panel).findByRole("tab", { name: "유사" }))
  return panel
}

describe("model gating (P3)", () => {
  it("T-ui-ai-models-01 [Business rule] e5 ready + CLIP missing enables the semantic switch but the similar tab shows a download notice", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 500, ai: { e5Ready: true } })
    await api.updateSettings({ hideShortcut: null }) // 설정 닫기 Esc 를 쓰려고 숨기기 단축키를 끈다
    const image = await findAsset(api, "image")
    const downloadSpy = vi.spyOn(api, "downloadModel")
    const user = userEvent.setup()
    renderApp(api)
    await screen.findAllByRole("option")

    // Act 1: 설정 > AI
    const dialog = await openSettingsPage(user, "로컬 AI·검색")
    const semantic = await within(dialog).findByRole("switch", { name: "의미 검색" })
    // Assert 1: CLIP 이 없어도 e5 만으로 켤 수 있다
    await waitFor(() => expect(semantic).toBeEnabled())
    await user.keyboard("{Escape}")

    // Act 2: 유사 탭
    const panel = await openSimilar(user, image.assetId)
    // Assert 2: 숨기지 않고 이유 + 내려받기 동작을 보여 준다
    expect(await within(panel).findByText("유사 미디어를 쓰려면 CLIP 모델이 필요합니다")).toBeInTheDocument()
    expect(within(panel).queryByRole("list", { name: "유사 미디어" })).not.toBeInTheDocument()
    await user.click(within(panel).getByRole("button", { name: "CLIP 모델 내려받기" }))
    await waitFor(() => expect(downloadSpy).toHaveBeenCalledTimes(2))
    expect(downloadSpy.mock.calls.map(([id]) => id).sort()).toEqual(["clip-image", "clip-text"])
  })

  it("T-ui-ai-models-02 [Business rule] e5 missing keeps the semantic switch disabled; CLIP ready + semantic off shows the enable notice", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 500, ai: { clipReady: true } })
    await api.updateSettings({ hideShortcut: null }) // 설정 닫기 Esc 를 쓰려고 숨기기 단축키를 끈다
    const image = await findAsset(api, "image")
    const user = userEvent.setup()
    renderApp(api)
    await screen.findAllByRole("option")

    // Act 1: 설정에서 스위치 확인
    const dialog = await openSettingsPage(user, "로컬 AI·검색")
    const semantic = await within(dialog).findByRole("switch", { name: "의미 검색" })
    // Assert 1: CLIP 만 준비돼도 의미 검색은 꺼 둔 채
    expect(semantic).toBeDisabled()
    await user.keyboard("{Escape}")

    // Act 2: 유사 탭
    const panel = await openSimilar(user, image.assetId)
    // Assert 2
    expect(await within(panel).findByText("의미 검색이 꺼져 있습니다")).toBeInTheDocument()
    expect(within(panel).queryByRole("list", { name: "유사 미디어" })).not.toBeInTheDocument()
    expect(within(panel).queryByText("유사 미디어를 쓰려면 CLIP 모델이 필요합니다")).not.toBeInTheDocument()
  })
})
