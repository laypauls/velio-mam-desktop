// Categories: Happy (T-ui-ai-detail-01, T-ui-ai-detail-03) / External (T-ui-ai-detail-02)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { sampleFor } from "@/lib/api/mock-ai-samples"
import { useSelectionStore, useViewStore } from "@/stores"
import { findAsset, immediate } from "@/test/ai-helpers"
import { renderApp, stubLayout } from "@/test/layout"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/** 자산을 선택하고 상세 패널의 AI 탭을 연다. */
async function openAiDetail(user: ReturnType<typeof userEvent.setup>, assetId: number) {
  act(() => useSelectionStore.getState().selectOnly(assetId))
  const panel = screen.getByRole("complementary", { name: "상세 정보 패널" })
  await user.click(await within(panel).findByRole("tab", { name: "AI" }))
  return panel
}

describe("detail panel ai tab (P3)", () => {
  it("T-ui-ai-detail-01 [Happy] edit description/keywords shows 편집됨, re-analyze asks for confirmation then calls analyzeAssets(force=true)", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 500, ai: { consent: true, schedule: immediate } })
    const image = await findAsset(api, "image")
    await api.setApiKey("claude", "sk-test", false)
    await api.analyzeAssets([image.assetId], null, false)
    const updateSpy = vi.spyOn(api, "updateAnalysis")
    const analyzeSpy = vi.spyOn(api, "analyzeAssets")
    const user = userEvent.setup()
    renderApp(api)
    const panel = await openAiDetail(user, image.assetId)
    expect(await within(panel).findByText(sampleFor(image.assetId).description)).toBeInTheDocument()
    expect(within(panel).queryByText("편집됨")).not.toBeInTheDocument()

    // Act 1: 편집 저장
    await user.click(within(panel).getByRole("button", { name: "편집" }))
    const description = within(panel).getByRole("textbox", { name: "설명" })
    await user.clear(description)
    await user.type(description, "새 설명")
    const keywords = within(panel).getByRole("textbox", { name: "키워드 (쉼표로 구분)" })
    await user.clear(keywords)
    await user.type(keywords, "가, 나")
    await user.click(within(panel).getByRole("button", { name: "저장" }))
    // Assert 1
    expect(await within(panel).findByText("새 설명")).toBeInTheDocument()
    expect(updateSpy).toHaveBeenCalledWith(image.assetId, { description: "새 설명", keywords: ["가", "나"] })
    expect(await within(panel).findByText("편집됨")).toBeInTheDocument()
    expect(within(panel).getByText("재분석하면 편집한 내용이 덮어써집니다.")).toBeInTheDocument()

    // Act 2: 재분석 — 경고 확인 전에는 호출하지 않는다
    await user.click(within(panel).getByRole("button", { name: "재분석" }))
    const confirm = await screen.findByRole("alertdialog", { name: "재분석할까요?" })
    expect(analyzeSpy).not.toHaveBeenCalled()
    await user.click(within(confirm).getByRole("button", { name: "재분석" }))
    // Assert 2
    await waitFor(() => expect(analyzeSpy).toHaveBeenCalledWith([image.assetId], null, true))
    await waitFor(() => expect(within(panel).queryByText("편집됨")).not.toBeInTheDocument())
    expect(await within(panel).findByText(sampleFor(image.assetId).description)).toBeInTheDocument()
  })

  it("T-ui-ai-detail-02 [External] analyzing without consent opens the consent dialog; declining does not retry, agreeing retries once", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 500, ai: { schedule: immediate } })
    const image = await findAsset(api, "image")
    await api.setApiKey("claude", "sk-test", false)
    const analyzeSpy = vi.spyOn(api, "analyzeAssets")
    const consentSpy = vi.spyOn(api, "giveAiConsent")
    const user = userEvent.setup()
    renderApp(api)
    const panel = await openAiDetail(user, image.assetId)
    await within(panel).findByText("아직 분석 결과가 없습니다")

    // Act 1: 분석 → 동의 Dialog → 동의하지 않음
    await user.click(within(panel).getByRole("button", { name: "분석" }))
    const dialog = await screen.findByRole("dialog", { name: "외부 AI 로 전송하는 데 동의하시겠습니까?" })
    expect(within(dialog).getByText(/원본 음성/)).toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "동의하지 않음" }))
    // Assert 1
    await waitFor(() => expect(screen.queryByRole("dialog", { name: /동의하시겠습니까/ })).not.toBeInTheDocument())
    expect(analyzeSpy).toHaveBeenCalledTimes(1)
    expect(consentSpy).not.toHaveBeenCalled()
    expect((await api.getAiSettings()).consent.given).toBe(false)

    // Act 2: 다시 분석 → 동의하고 계속 → 한 번 재시도
    await user.click(within(panel).getByRole("button", { name: "분석" }))
    const again = await screen.findByRole("dialog", { name: /동의하시겠습니까/ })
    await user.click(within(again).getByRole("button", { name: "동의하고 계속" }))
    // Assert 2
    await waitFor(() => expect(analyzeSpy).toHaveBeenCalledTimes(3))
    expect(consentSpy).toHaveBeenCalledTimes(1)
    expect((await api.getAiSettings()).consent.given).toBe(true)
    expect(await within(panel).findByText(sampleFor(image.assetId).description)).toBeInTheDocument()
  })

  it("T-ui-ai-detail-03 [Happy] clicking an STT segment opens the preview player at that time, and seeks it while open", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 500, ai: { consent: true, schedule: immediate } })
    const video = await findAsset(api, "video", { playable: true })
    await api.analyzeAssets([video.assetId], ["stt"], false)
    const seeks: number[] = []
    vi.spyOn(HTMLMediaElement.prototype, "currentTime", "set").mockImplementation((v: number) => void seeks.push(v))
    const user = userEvent.setup()
    renderApp(api)
    const panel = await openAiDetail(user, video.assetId)
    expect(await within(panel).findByText("음성 인식 · 한국어")).toBeInTheDocument()
    const segments = within(panel).getByRole("list", { name: "음성 인식 구간" })
    const second = within(segments).getAllByRole("button")[1]

    // Act 1: 닫혀 있는 큰 보기 → 해당 시각으로 연다
    await user.click(second)
    // Assert 1
    await waitFor(() => expect(document.querySelector("video")).not.toBeNull())
    await waitFor(() => expect(seeks).toContain(7))
    expect(useViewStore.getState().previewId).toBe(video.assetId)

    // Act 2: 이미 열려 있으면 그 자리에서 이동
    act(() => useViewStore.getState().seekPreview(video.assetId, 14_000))
    // Assert 2
    await waitFor(() => expect(seeks).toContain(14))
    expect(document.querySelectorAll("video")).toHaveLength(1)
  })
})
