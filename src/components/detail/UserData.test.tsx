// Categories: Happy (T-ui-user-01) / Validation + Business rule (T-ui-user-02)
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { optionNamed } from "@/test/queries"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

type User = ReturnType<typeof userEvent.setup>

/** 첫 N개 항목의 파일명을 읽는다 (카드 이름은 `파일명, 유형…`). */
async function optionNames(n: number): Promise<string[]> {
  const options = await screen.findAllByRole("option")
  return options.slice(0, n).map((o) => (o.getAttribute("aria-label") as string).split(", ")[0])
}

async function openDetail(user: User, name: string) {
  await user.click(screen.getByRole("option", { name: optionNamed(name) }))
  const panel = screen.getByRole("complementary", { name: "상세 정보 패널" })
  await within(panel).findByRole("heading", { level: 2, name })
  return panel
}

describe("user data (P2-7)", () => {
  it("T-ui-user-01 [Happy] tag add/remove, memo save, favorite toggle and bulk tag change mock state and refresh the UI", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 500 })
    const user = userEvent.setup()
    renderApp(api)
    const [first, second] = await optionNames(2)
    const idOf = (name: string) => Number((screen.getByRole("option", { name: optionNamed(name) }) as HTMLElement).dataset.assetId)
    const firstId = idOf(first)
    const secondId = idOf(second)
    const detail = (id: number) => api.getAssetDetail(id)
    const panel = await openDetail(user, first)

    // Act + Assert: 태그 추가 (Enter)
    await user.type(within(panel).getByRole("textbox", { name: "태그 추가" }), "신규태그{Enter}")
    const tagList = await within(panel).findByRole("list", { name: "태그" })
    expect(within(tagList).getByText("신규태그")).toBeInTheDocument()
    expect((await detail(firstId)).tags.map((t) => t.name)).toContain("신규태그")

    // Act + Assert: 태그 제거
    await user.click(within(panel).getByRole("button", { name: "태그 신규태그 제거" }))
    await waitFor(() => expect(within(panel).queryByText("신규태그")).not.toBeInTheDocument())
    expect((await detail(firstId)).tags.map((t) => t.name)).not.toContain("신규태그")

    // Act + Assert: 메모 (blur 로 저장)
    const memo = within(panel).getByRole("textbox", { name: "메모" })
    await user.clear(memo)
    await user.type(memo, "테스트 메모")
    await user.tab()
    expect(await within(panel).findByText("저장됨")).toBeInTheDocument()
    expect((await detail(firstId)).memo).toBe("테스트 메모")

    // Act + Assert: 즐겨찾기 토글
    const before = (await detail(firstId)).favorite
    const fav = within(panel).getByRole("button", { name: "즐겨찾기" })
    expect(fav).toHaveAttribute("aria-pressed", String(before))
    await user.click(fav)
    await waitFor(() => expect(within(panel).getByRole("button", { name: "즐겨찾기" })).toHaveAttribute("aria-pressed", String(!before)))
    expect((await detail(firstId)).favorite).toBe(!before)

    // Act + Assert: 일괄 태그 (2개 선택 → 태그 Dialog)
    await user.keyboard("{Control>}")
    await user.click(screen.getByRole("option", { name: optionNamed(second) }))
    await user.keyboard("{/Control}")
    const bar = await screen.findByRole("toolbar", { name: "선택 항목 일괄 작업" })
    await user.click(within(bar).getByRole("button", { name: "태그" }))
    const dialog = await screen.findByRole("dialog", { name: "일괄 태그" })
    await user.type(within(dialog).getByRole("textbox", { name: "태그 추가" }), "일괄태그{Enter}")
    await user.click(within(dialog).getByRole("button", { name: "적용" }))
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "일괄 태그" })).not.toBeInTheDocument())
    expect((await detail(firstId)).tags.map((t) => t.name)).toContain("일괄태그")
    expect((await detail(secondId)).tags.map((t) => t.name)).toContain("일괄태그")
  })

  it("T-ui-user-02a [Validation] empty collection name is rejected inline with aria-invalid and does not call the command", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    const create = vi.spyOn(api, "createCollection")
    const user = userEvent.setup()
    renderApp(api)
    // Act: 컬렉션 섹션으로 전환 (레일의 독립 섹션)
    await user.click(await screen.findByRole("button", { name: "컬렉션" }))
    await user.click(await screen.findByRole("button", { name: "컬렉션 추가" }))
    const dialog = await screen.findByRole("dialog", { name: "새 컬렉션" })
    const input = within(dialog).getByRole("textbox", { name: "컬렉션 이름" })
    await user.click(within(dialog).getByRole("button", { name: "만들기" }))
    // Assert
    expect(input).toHaveAttribute("aria-invalid", "true")
    const message = within(dialog).getByRole("alert")
    expect(message).toHaveTextContent("컬렉션 이름을 입력하세요")
    expect(input.getAttribute("aria-describedby")).toBe(message.id)
    expect(create).not.toHaveBeenCalled()
    // Act: 이름 입력 후 만들기
    await user.type(input, "새 모음")
    await user.click(within(dialog).getByRole("button", { name: "만들기" }))
    // Assert
    expect(create).toHaveBeenCalledWith("새 모음")
    const nav = screen.getByRole("navigation", { name: "라이브러리 탐색" })
    expect(await within(nav).findByRole("button", { name: /^새 모음/ })).toBeInTheDocument()
  })

  it("T-ui-user-02b [Validation] empty tag name is rejected in the tag manager and the detail tag input", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    const rename = vi.spyOn(api, "renameTag")
    const update = vi.spyOn(api, "updateAssetTags")
    const user = userEvent.setup()
    renderApp(api, { section: "tags" })
    // Act: 태그 관리 → 이름 변경에서 비우기
    await user.click(await screen.findByRole("button", { name: "태그 관리" }))
    const dialog = await screen.findByRole("dialog", { name: "태그 관리" })
    await user.click(await within(dialog).findByRole("button", { name: "태그 여행 이름 변경" }))
    const field = within(dialog).getByRole("textbox", { name: "여행 새 이름" })
    await user.clear(field)
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    // Assert
    expect(field).toHaveAttribute("aria-invalid", "true")
    expect(within(dialog).getByRole("alert")).toHaveTextContent("태그 이름을 입력하세요")
    expect(rename).not.toHaveBeenCalled()
    // Act: 닫고 상세 패널 태그 입력에서 빈 Enter
    await user.keyboard("{Escape}")
    const [first] = await optionNames(1)
    const panel = await openDetail(user, first)
    const tagInput = within(panel).getByRole("textbox", { name: "태그 추가" })
    await user.type(tagInput, "{Enter}")
    // Assert
    expect(tagInput).toHaveAttribute("aria-invalid", "true")
    expect(update).not.toHaveBeenCalled()
  })

  it("T-ui-user-02c [Business rule] deleting a collection asks for confirmation, states media is kept, and assets remain", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    const del = vi.spyOn(api, "deleteCollection")
    const user = userEvent.setup()
    renderApp(api)
    const { totalAssets } = await api.jobSummary()
    const nav = await screen.findByRole("navigation", { name: "라이브러리 탐색" })
    await user.click(await screen.findByRole("button", { name: "컬렉션" }))
    await within(nav).findByRole("button", { name: /^포트폴리오/ })
    // Act: 메뉴 → 삭제
    await user.click(within(nav).getByRole("button", { name: "컬렉션 포트폴리오 메뉴" }))
    await user.click(await screen.findByRole("menuitem", { name: "삭제" }))
    // Assert: 확인 단계, 아직 호출 안 됨
    const confirm = await screen.findByRole("alertdialog")
    expect(confirm).toHaveTextContent("미디어 파일과 자산은 삭제되지 않습니다")
    expect(del).not.toHaveBeenCalled()
    // Act: 확인
    await user.click(within(confirm).getByRole("button", { name: "컬렉션 삭제" }))
    // Assert
    await waitFor(() => expect(del).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(within(nav).queryByRole("button", { name: /^포트폴리오/ })).not.toBeInTheDocument())
    expect((await api.jobSummary()).totalAssets).toBe(totalAssets)
    expect(screen.getAllByRole("option").length).toBeGreaterThan(0)
  })
})
