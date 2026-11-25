// Categories: Happy (T-ui-reuse-01) / External failure + Business rule (T-ui-reuse-02)
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { ExportProgress } from "@/lib/ipc-contract"
import { renderApp, stubLayout } from "@/test/layout"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

type User = ReturnType<typeof userEvent.setup>

/** 진행률 이벤트를 테스트가 직접 내보낼 수 있는 내보내기 mock. */
function controllableExport(api: ReturnType<typeof createMockApi>) {
  let handler: ((p: ExportProgress) => void) | null = null
  const exportSpy = vi.spyOn(api, "exportAssets").mockResolvedValue({ taskId: "t1" })
  vi.spyOn(api, "onExportProgress").mockImplementation(async (h) => {
    handler = h
    return () => {
      handler = null
    }
  })
  const emit = (p: Partial<ExportProgress>) =>
    act(() => handler?.({ taskId: "t1", done: 0, total: 2, failed: 0, skipped: 0, finished: false, ...p }))
  return { exportSpy, emit }
}

async function selectTwoAndOpenExport(user: User) {
  const options = await screen.findAllByRole("option")
  await user.click(options[0])
  await user.keyboard("{Control>}")
  await user.click(options[1])
  await user.keyboard("{/Control}")
  const bar = await screen.findByRole("toolbar", { name: "선택 항목 일괄 작업" })
  await user.click(within(bar).getByRole("button", { name: "내보내기" }))
  return screen.findByRole("dialog", { name: "내보내기" })
}

describe("reuse (P2-8)", () => {
  it("T-ui-reuse-01 [Happy] export: pick folder -> progress -> done with finish toast", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    const pick = vi.spyOn(api, "pickDirectory").mockResolvedValue("D:\\Export")
    const { exportSpy, emit } = controllableExport(api)
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await selectTwoAndOpenExport(user)
    const start = within(dialog).getByRole("button", { name: "내보내기 시작" })
    expect(start).toBeDisabled() // 폴더를 고르기 전에는 시작할 수 없다
    // Act: 폴더 선택 → 시작
    await user.click(within(dialog).getByRole("button", { name: "폴더 선택" }))
    expect(await within(dialog).findByText("D:\\Export")).toBeInTheDocument()
    await user.click(start)
    // Assert: command 인자 + 진행 화면
    await waitFor(() => expect(exportSpy).toHaveBeenCalledTimes(1))
    expect(pick).toHaveBeenCalledTimes(1)
    const input = exportSpy.mock.calls[0][0]
    expect(input).toMatchObject({ destDir: "D:\\Export", conflict: "rename", includeManifest: true })
    expect(input.assetIds).toHaveLength(2)
    const bar = await within(dialog).findByRole("progressbar", { name: "내보내기 진행률" })
    // Act: 진행 이벤트
    await emit({ done: 1 })
    // Assert
    expect(await within(dialog).findByText("1 / 2개 처리")).toBeInTheDocument()
    expect(bar).toBeInTheDocument()
    // Act: 완료
    await emit({ done: 2, finished: true })
    // Assert
    expect(await screen.findByRole("dialog", { name: "내보내기 완료" })).toBeInTheDocument()
    expect(await screen.findByText(/내보내기를 마쳤습니다\. 성공 2개/)).toBeInTheDocument()
    expect(within(dialog).getByText("2 / 2개 처리")).toBeInTheDocument()
    expect(within(dialog).queryByRole("alert")).not.toBeInTheDocument()
  })

  it("T-ui-reuse-02a [External failure] cancelled folder pick does nothing and export cannot start", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    vi.spyOn(api, "pickDirectory").mockResolvedValue(null)
    const { exportSpy } = controllableExport(api)
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await selectTwoAndOpenExport(user)
    // Act
    await user.click(within(dialog).getByRole("button", { name: "폴더 선택" }))
    // Assert
    expect(within(dialog).getByText("선택된 폴더 없음")).toBeInTheDocument()
    expect(within(dialog).getByRole("button", { name: "내보내기 시작" })).toBeDisabled()
    expect(within(dialog).queryByRole("alert")).not.toBeInTheDocument()
    expect(exportSpy).not.toHaveBeenCalled()
  })

  it("T-ui-reuse-02b [External failure] shows the failed count in progress, final notice and warning toast", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    vi.spyOn(api, "pickDirectory").mockResolvedValue("D:\\Export")
    const { emit } = controllableExport(api)
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await selectTwoAndOpenExport(user)
    await user.click(within(dialog).getByRole("button", { name: "폴더 선택" }))
    await user.click(await within(dialog).findByRole("button", { name: "내보내기 시작" }))
    await within(dialog).findByRole("progressbar")
    // Act
    await emit({ done: 1, failed: 1 })
    // Assert: 진행 중 실패 건수
    expect(await within(dialog).findByText(/실패 1개/)).toBeInTheDocument()
    // Act
    await emit({ done: 2, failed: 1, finished: true })
    // Assert
    const alert = await within(dialog).findByRole("alert")
    expect(alert).toHaveTextContent("1개 파일을 내보내지 못했습니다")
    expect(await screen.findByText("내보내기를 마쳤지만 1개를 내보내지 못했습니다.")).toBeInTheDocument()
  })

  it("T-ui-reuse-02c [Business rule] open/reveal are disabled with a reason for offline assets and call the command for online ones", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    const original = api.getAssetDetail.bind(api)
    const open = vi.spyOn(api, "openOriginal")
    const reveal = vi.spyOn(api, "revealInExplorer")
    const copy = vi.spyOn(api, "copyToClipboard")
    let offline = true
    vi.spyOn(api, "getAssetDetail").mockImplementation(async (id) => ({
      ...(await original(id)),
      availability: offline ? "offline" : "online",
    }))
    const user = userEvent.setup()
    renderApp(api)
    const options = await screen.findAllByRole("option")
    // Act: 오프라인 자산 선택
    await user.click(options[0])
    const panel = screen.getByRole("complementary", { name: "상세 정보 패널" })
    const openBtn = await within(panel).findByRole("button", { name: "열기" })
    const revealBtn = within(panel).getByRole("button", { name: "탐색기에서 보기" })
    // Assert
    expect(openBtn).toBeDisabled()
    expect(revealBtn).toBeDisabled()
    act(() => openBtn.parentElement?.focus())
    expect(await screen.findByRole("tooltip")).toHaveTextContent("저장 장치가 연결되어 있지 않아")
    expect(open).not.toHaveBeenCalled()
    // Act: 정상 자산 선택 후 열기/탐색기/복사
    offline = false
    await user.click(options[1])
    await waitFor(() => expect(within(panel).getByRole("button", { name: "열기" })).toBeEnabled())
    await user.click(within(panel).getByRole("button", { name: "열기" }))
    await user.click(within(panel).getByRole("button", { name: "탐색기에서 보기" }))
    // 패널 폭이 충분하면 파일 복사도 인라인 아이콘 버튼이다 (⋯ 메뉴는 넘칠 때만)
    await user.click(within(panel).getByRole("button", { name: "파일을 클립보드에 복사" }))
    // Assert
    const id = Number(options[1].getAttribute("data-asset-id"))
    await waitFor(() => expect(copy).toHaveBeenCalledWith([id]))
    expect(open).toHaveBeenCalledWith(id)
    expect(reveal).toHaveBeenCalledWith(id)
  })
})

describe("gallery context menu (P2-8)", () => {
  it("T-ui-reuse-03 [Happy] right-click opens the item menu; export opens the dialog and favorite changes the mock state", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    const fav = vi.spyOn(api, "setFavorite")
    const user = userEvent.setup()
    renderApp(api)
    const option = (await screen.findAllByRole("option"))[0]
    const id = Number(option.getAttribute("data-asset-id"))
    // Act
    fireEvent.contextMenu(option)
    // Assert: 메뉴 항목
    for (const name of ["미리보기 열기", "열기", "탐색기에서 보기", "내보내기"]) {
      expect(await screen.findByRole("menuitem", { name: new RegExp(`^${name}`) })).toBeInTheDocument()
    }
    expect(screen.getByRole("menuitem", { name: "컬렉션에 추가" })).toBeInTheDocument()
    // Act: 즐겨찾기
    const favItem = screen.getByRole("menuitem", { name: /^즐겨찾기/ })
    const wasFav = favItem.textContent === "즐겨찾기 해제"
    await user.click(favItem)
    // Assert
    await waitFor(() => expect(fav).toHaveBeenCalledWith([id], !wasFav))
    // Act: 다시 열어 내보내기
    fireEvent.contextMenu(screen.getAllByRole("option")[0])
    await user.click(await screen.findByRole("menuitem", { name: "내보내기" }))
    // Assert
    expect(await screen.findByRole("dialog", { name: "내보내기" })).toBeInTheDocument()
  })
})
