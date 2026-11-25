// Categories: Happy (T-ui-open-01, T-ui-open-02, T-ui-open-05, T-ui-title-01, T-ui-title-03, T-ui-title-06) / Business rule (T-ui-open-03, T-ui-open-06, T-ui-open-07, T-ui-open-08, T-ui-title-02, T-ui-title-05, T-ui-title-07) / External (T-ui-open-04) / Validation (T-ui-title-04)
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { mockFixedResults } from "@/test/searchMock"
import type { AssetSummary, SettingsPatch } from "@/lib/ipc-contract"
import { useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { optionNamed } from "@/test/queries"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const asset = (id: number, over: Partial<AssetSummary> = {}): AssetSummary => ({
  assetId: id,
  fileName: `file_${id}.jpg`,
  title: null,
  mediaType: "image",
  availability: "online",
  width: 100,
  height: 100,
  durationMs: null,
  fileSize: 1000,
  capturedAt: Date.UTC(2026, 9, 15),
  modifiedAt: Date.UTC(2026, 9, 15),
  favorite: false,
  rating: 0,
  thumbState: "done",
  aiState: "none",
  revision: 1,
  ...over,
})

/** 1: 온라인(제목 있음) / 2: 오프라인 / 3: 파일 없음 */
const ITEMS = [asset(1, { title: "제주 일출" }), asset(2, { availability: "offline" }), asset(3, { availability: "missing" })]

async function closePreview() {
  act(() => useViewStore.getState().closePreview())
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
}

async function setup(settings: SettingsPatch = {}) {
  stubLayout()
  const api = createMockApi({ count: 10 })
  await api.updateSettings(settings)
  mockFixedResults(api, ITEMS)
  const open = vi.spyOn(api, "openOriginal").mockResolvedValue(undefined)
  const user = userEvent.setup()
  renderApp(api)
  await screen.findAllByRole("option")
  return { api, user, open }
}

describe("display name (title vs file name)", () => {
  it("T-ui-title-01 [Happy] title edit ON shows the title in cards (file name when no title) with a title + (file name) tooltip", async () => {
    // Arrange + Act
    await setup({ titleEditEnabled: true })
    // Assert
    expect(await screen.findByRole("option", { name: optionNamed("제주 일출") })).toBeInTheDocument()
    expect(screen.getByRole("option", { name: optionNamed("file_2.jpg") })).toBeInTheDocument() // 제목 없음 -> 파일명
    expect(screen.getByTitle("제주 일출 (file_1.jpg)")).toBeInTheDocument()
  })

  it("T-ui-title-02 [Business rule] title edit OFF keeps showing the file name even though a title is stored", async () => {
    // Arrange + Act
    await setup({ titleEditEnabled: false })
    // Assert
    expect(await screen.findByRole("option", { name: optionNamed("file_1.jpg") })).toBeInTheDocument()
    expect(screen.queryByRole("option", { name: optionNamed("제주 일출") })).not.toBeInTheDocument()
  })
})

describe("open asset (double-click and Enter)", () => {
  it("T-ui-open-01 [Happy] preview mode: double-click and Enter open the preview and never call openOriginal", async () => {
    // Arrange
    const { user, open } = await setup({ doubleClickAction: "preview" })
    // Act + Assert: 더블클릭
    await user.dblClick(screen.getByRole("option", { name: optionNamed("file_1.jpg") }))
    expect(useViewStore.getState().previewId).toBe(1)
    // Act + Assert: Enter
    await closePreview()
    await user.click(screen.getByRole("option", { name: optionNamed("file_1.jpg") }))
    await user.keyboard("{Enter}")
    expect(useViewStore.getState().previewId).toBe(1)
    expect(open).not.toHaveBeenCalled()
  })

  it("T-ui-open-02 [Happy] openSystem mode: double-click and Enter call openOriginal for an online asset without opening the preview", async () => {
    // Arrange
    const { user, open } = await setup({ doubleClickAction: "openSystem" })
    // Act
    await user.dblClick(screen.getByRole("option", { name: optionNamed("file_1.jpg") }))
    // Assert
    await waitFor(() => expect(open).toHaveBeenCalledWith(1))
    expect(useViewStore.getState().previewId).toBeNull()
    // Act: Enter (포커스 항목 = 클릭한 항목)
    await user.keyboard("{Enter}")
    await waitFor(() => expect(open).toHaveBeenCalledTimes(2))
    expect(useViewStore.getState().previewId).toBeNull()
  })

  it("T-ui-open-03 [Business rule] openSystem mode never opens the preview or calls openOriginal for offline / missing assets and shows the reason toast", async () => {
    // Arrange
    const { user, open } = await setup({ doubleClickAction: "openSystem" })
    // Act: 오프라인
    await user.dblClick(screen.getByRole("option", { name: optionNamed("file_2.jpg") }))
    // Assert
    expect(useViewStore.getState().previewId).toBeNull()
    expect(await screen.findByText(/저장 장치가 연결되어 있지 않아/)).toBeInTheDocument()
    // Act: 파일 없음 (Enter)
    await user.click(screen.getByRole("option", { name: optionNamed("file_3.jpg") }))
    await user.keyboard("{Enter}")
    // Assert
    expect(await screen.findByText(/원본 파일을 찾을 수 없어/)).toBeInTheDocument()
    expect(useViewStore.getState().previewId).toBeNull()
    expect(open).not.toHaveBeenCalled()
  })

  it("T-ui-open-04 [External] openSystem failure falls back to the preview and shows the error reason", async () => {
    // Arrange
    const { user, open } = await setup({ doubleClickAction: "openSystem" })
    open.mockRejectedValueOnce({ code: "io", message: "launch failed" })
    // Act
    await user.dblClick(screen.getByRole("option", { name: optionNamed("file_1.jpg") }))
    // Assert
    await waitFor(() => expect(useViewStore.getState().previewId).toBe(1))
    expect((await screen.findAllByText(/미리보기로 열었습니다/)).length).toBeGreaterThan(0)
  })
  it("T-ui-open-05 [Happy] online asset with preview setting opens the preview", async () => {
    // Arrange
    const { user } = await setup({ doubleClickAction: "preview" })
    // Act
    await user.dblClick(screen.getByRole("option", { name: optionNamed("file_1.jpg") }))
    // Assert
    expect(useViewStore.getState().previewId).toBe(1)
  })

  it("T-ui-open-06 [Business rule] preview setting: offline and missing assets (images included) do not open the preview and toast the reason", async () => {
    // Arrange
    const { user } = await setup({ doubleClickAction: "preview" })
    // Act
    await user.dblClick(screen.getByRole("option", { name: optionNamed("file_2.jpg") }))
    await user.click(screen.getByRole("option", { name: optionNamed("file_3.jpg") }))
    await user.keyboard("{Enter}")
    // Assert
    expect(useViewStore.getState().previewId).toBeNull()
    expect(await screen.findByText(/저장 장치가 연결되어 있지 않아/)).toBeInTheDocument()
    expect(await screen.findByText(/원본 파일을 찾을 수 없어/)).toBeInTheDocument()
  })

  it("T-ui-open-07 [Business rule] card quick action 큰 보기 does not open the preview for offline cards but does for online cards", async () => {
    // Arrange
    const { user } = await setup()
    const quick = (name: string) => within(screen.getByRole("option", { name: optionNamed(name) })).getByRole("button", { name: "큰 보기" })
    // Act: 오프라인
    await user.click(quick("file_2.jpg"))
    // Assert
    expect(useViewStore.getState().previewId).toBeNull()
    expect(await screen.findByText(/저장 장치가 연결되어 있지 않아/)).toBeInTheDocument()
    // Act: 온라인
    await user.click(quick("file_1.jpg"))
    // Assert
    expect(useViewStore.getState().previewId).toBe(1)
  })

  it("T-ui-open-08 [Business rule] context menu 미리보기 열기 is disabled for offline / missing and enabled for online", async () => {
    // Arrange
    await setup()
    const itemFor = async (name: string) => {
      fireEvent.contextMenu(screen.getByRole("option", { name: optionNamed(name) }))
      const item = await screen.findByRole("menuitem", { name: /미리보기 열기/ })
      return item
    }
    // Act + Assert
    expect(await itemFor("file_2.jpg")).toHaveAttribute("aria-disabled", "true")
    act(() => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })) })
    await waitFor(() => expect(screen.queryByRole("menuitem", { name: /미리보기 열기/ })).not.toBeInTheDocument())
    expect(await itemFor("file_3.jpg")).toHaveAttribute("aria-disabled", "true")
    act(() => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })) })
    await waitFor(() => expect(screen.queryByRole("menuitem", { name: /미리보기 열기/ })).not.toBeInTheDocument())
    expect(await itemFor("file_1.jpg")).not.toHaveAttribute("aria-disabled", "true")
  })
})

describe("title editing UI", () => {
  async function openDetailWithTitles() {
    stubLayout()
    const api = createMockApi({ count: 30 })
    await api.updateSettings({ titleEditEnabled: true })
    const user = userEvent.setup()
    renderApp(api)
    const first = (await screen.findAllByRole("option"))[0]
    const id = Number(first.dataset.assetId)
    await user.click(first)
    const panel = screen.getByRole("complementary", { name: "상세 정보 패널" })
    const enter = async () => {
      await user.click(await within(panel).findByRole("button", { name: "제목 편집" }))
      return within(panel).getByRole("textbox", { name: "제목" }) as HTMLInputElement
    }
    const input = await enter()
    const fileName = (await api.getAssetDetail(id)).fileName
    return { api, user, panel, input, enter, id, fileName }
  }

  it("T-ui-title-03 [Happy] detail: Enter saves, unchanged blur makes no call, Esc reverts, clear button removes the title", async () => {
    // Arrange
    const { api, user, panel, input, enter, id, fileName } = await openDetailWithTitles()
    const spy = vi.spyOn(api, "setAssetTitle")
    expect(within(panel).getByText(fileName)).toBeInTheDocument() // 실제 파일명은 항상 표시
    expect(input).toHaveAttribute("maxlength", "200")
    // Act: 저장 (Enter)
    await user.clear(input)
    await user.type(input, "  새 제목  {Enter}")
    // Assert
    await waitFor(() => expect(spy).toHaveBeenCalledWith(id, "새 제목"))
    expect((await api.getAssetDetail(id)).title).toBe("새 제목")
    await waitFor(() => expect(screen.getByRole("option", { name: optionNamed("새 제목") })).toBeInTheDocument()) // 목록 갱신
    await waitFor(() => expect(within(panel).queryByRole("textbox", { name: "제목" })).not.toBeInTheDocument()) // 보기 상태로 복귀
    // Act: 변경 없이 blur -> 호출 없음
    spy.mockClear()
    const again = await enter()
    await user.click(again)
    await user.tab()
    expect(spy).not.toHaveBeenCalled()
    // Act: Esc 되돌리기
    const third = await enter()
    await user.type(third, "버릴 내용")
    await user.keyboard("{Escape}")
    expect(await enter()).toHaveValue("새 제목")
    await user.tab()
    expect(spy).not.toHaveBeenCalled()
    // Act: 해제 버튼
    await enter()
    await user.click(within(panel).getByRole("button", { name: "제목 지우기 (파일명 사용)" }))
    await waitFor(() => expect(spy).toHaveBeenCalledWith(id, null))
    expect((await api.getAssetDetail(id)).title).toBeNull()
    expect(await enter()).toHaveValue(fileName.replace(/.[^.]+$/, "")) // 해제 후 기본값(확장자 없는 파일명)
  })

  it("T-ui-title-04 [Validation] a rejected save shows an inline error tied to the input and keeps the typed value", async () => {
    // Arrange
    const { api, user, panel, input } = await openDetailWithTitles()
    vi.spyOn(api, "setAssetTitle").mockRejectedValue({ code: "invalid_input", message: "title too long" })
    // Act
    await user.clear(input)
    await user.type(input, "나쁜 제목{Enter}")
    // Assert
    const alert = await within(panel).findByRole("alert")
    expect(alert).toHaveTextContent("입력값이 올바르지 않습니다.")
    expect(input).toHaveAttribute("aria-invalid", "true")
    expect(input).toHaveAttribute("aria-describedby", alert.id)
    expect(input).toHaveValue("나쁜 제목")
  })

  it("T-ui-title-05 [Business rule] detail shows no title input when title edit is OFF (file name heading as before)", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 30 })
    const user = userEvent.setup()
    renderApp(api)
    // Act
    await user.click((await screen.findAllByRole("option"))[0])
    const panel = screen.getByRole("complementary", { name: "상세 정보 패널" })
    // Assert
    await within(panel).findAllByRole("heading", { level: 2 })
    expect(within(panel).queryByRole("textbox", { name: "제목" })).not.toBeInTheDocument()
  })

  it("T-ui-title-06 [Happy] context menu 제목 변경 saves via the dialog; a failed save stays inline and can be retried", async () => {
    // Arrange
    const { api, user } = await openDetailWithTitles()
    const target = (await screen.findAllByRole("option"))[1]
    const id = Number(target.dataset.assetId)
    // Act: 우클릭 메뉴
    fireEvent.contextMenu(target)
    await user.click(await screen.findByRole("menuitem", { name: "제목 변경…" }))
    const dialog = await screen.findByRole("dialog", { name: "제목 변경" })
    const field = within(dialog).getByRole("textbox", { name: "제목" })
    // Assert: 실패 -> 인라인 오류, 대화상자 유지
    vi.spyOn(api, "setAssetTitle").mockRejectedValueOnce({ code: "invalid_input", message: "x" })
    await user.clear(field)
    await user.type(field, "메뉴 제목")
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    expect(await within(dialog).findByRole("alert")).toBeInTheDocument()
    // Act: 재시도 성공
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "제목 변경" })).not.toBeInTheDocument())
    expect((await api.getAssetDetail(id)).title).toBe("메뉴 제목")
  })

  it("T-ui-title-07 [Business rule] context menu hides 제목 변경 when title edit is OFF", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 30 })
    renderApp(api)
    // Act
    fireEvent.contextMenu((await screen.findAllByRole("option"))[0])
    // Assert
    await screen.findByRole("menuitem", { name: /미리보기 열기/ })
    expect(screen.queryByRole("menuitem", { name: "제목 변경…" })).not.toBeInTheDocument()
  })
})
