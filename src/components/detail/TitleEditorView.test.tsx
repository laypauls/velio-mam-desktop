// Categories: Happy (T-ui-title-20..24, 28, 30, 31) / Business rule (T-ui-title-25, 27) / External (T-ui-title-26) / Concurrency (T-ui-title-29)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { TitleEditor } from "./TitleEditor"

afterEach(() => vi.restoreAllMocks())

async function setup(title: string | null, { enabled = true, fileName = "IMG_0612.jpg" } = {}) {
  const api = createMockApi({ count: 5 })
  await api.updateSettings({ titleEditEnabled: enabled })
  const spy = vi.spyOn(api, "setAssetTitle").mockResolvedValue(undefined)
  const user = userEvent.setup()
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const ui = (assetId: number, current: string | null = title) => (
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <TitleEditor assetId={assetId} title={current} fileName={fileName} />
      </QueryClientProvider>
    </ApiProvider>
  )
  const view = render(ui(1))
  // 설정 로딩이 끝나길 기다린다 (켜짐: ✎, 꺼짐: 파일명 제목)
  if (enabled) await screen.findByRole("button", { name: "제목 편집" })
  else await screen.findByRole("heading", { name: fileName })
  return { user, spy, rerender: (id: number, current: string | null = title) => view.rerender(ui(id, current)) }
}

const pencil = () => screen.getByRole("button", { name: "제목 편집" })
const field = () => screen.getByRole("textbox", { name: "제목" }) as HTMLInputElement

describe("title editor view/edit states", () => {
  it("T-ui-title-20 [Happy] view state shows no textbox, with and without a stored title", async () => {
    // Arrange / Act
    await setup("제주 일출")
    // Assert
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
    expect(screen.getByRole("heading", { name: "제주 일출" })).toBeInTheDocument()
    expect(screen.getByText("IMG_0612.jpg", { exact: false })).toBeInTheDocument()
  })

  it("T-ui-title-21 [Happy] without a title the file name appears once, with the pencil", async () => {
    await setup(null)
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
    expect(screen.getAllByText("IMG_0612.jpg")).toHaveLength(1)
    expect(screen.queryByText("파일명")).not.toBeInTheDocument()
    expect(pencil()).toBeInTheDocument()
  })

  it("T-ui-title-22 [Happy] pencil click focuses the input with the whole value selected", async () => {
    const { user } = await setup("제주 일출")
    await user.click(pencil())
    const input = field()
    expect(input).toHaveFocus()
    expect(input.selectionStart).toBe(0)
    expect(input.selectionEnd).toBe("제주 일출".length)
    expect(screen.getByText("Enter 저장 · Esc 취소")).toBeInTheDocument()
  })

  it("T-ui-title-23 [Happy] double-clicking the heading enters edit mode", async () => {
    const { user } = await setup(null)
    await user.dblClick(screen.getByRole("heading", { name: "IMG_0612.jpg" }))
    expect(field()).toHaveFocus()
    expect(field().selectionEnd).toBe("IMG_0612".length)
  })

  it("T-ui-title-24 [Happy] Enter saves and returns to the view state", async () => {
    const { user, spy } = await setup("제주 일출")
    await user.click(pencil())
    await user.clear(field())
    await user.type(field(), "새 제목{Enter}")
    await waitFor(() => expect(spy).toHaveBeenCalledWith(1, "새 제목"))
    await waitFor(() => expect(screen.queryByRole("textbox")).not.toBeInTheDocument())
  })

  it("T-ui-title-25 [Business rule] blur saves", async () => {
    const { user, spy } = await setup("제주 일출")
    await user.click(pencil())
    await user.type(field(), "!")
    await user.tab()
    await waitFor(() => expect(spy).toHaveBeenCalledWith(1, "제주 일출!"))
    await waitFor(() => expect(screen.queryByRole("textbox")).not.toBeInTheDocument())
  })

  it("T-ui-title-26 [External] a failed save keeps edit mode with an inline alert", async () => {
    const { user, spy } = await setup("제주 일출")
    spy.mockRejectedValue({ code: "invalid_input", message: "x" })
    await user.click(pencil())
    await user.type(field(), "!{Enter}")
    const alert = await screen.findByRole("alert")
    expect(field()).toHaveAttribute("aria-invalid", "true")
    expect(field()).toHaveAttribute("aria-describedby", alert.id)
  })

  it("T-ui-title-27 [Business rule] Esc does not save and returns focus to the pencil", async () => {
    const { user, spy } = await setup("제주 일출")
    await user.click(pencil())
    await user.type(field(), "버릴 내용{Escape}")
    expect(spy).not.toHaveBeenCalled()
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
    expect(pencil()).toHaveFocus()
  })

  it("T-ui-title-28 [Business rule] with title editing off there is no pencil and no textbox", async () => {
    await setup("제주 일출", { enabled: false })
    expect(screen.queryByRole("button", { name: "제목 편집" })).not.toBeInTheDocument()
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
  })

  it("T-ui-title-29 [Concurrency] changing the asset leaves edit mode", async () => {
    const { user, rerender } = await setup("제주 일출")
    await user.click(pencil())
    expect(field()).toBeInTheDocument()
    rerender(2)
    await waitFor(() => expect(screen.queryByRole("textbox")).not.toBeInTheDocument())
  })

  it("T-ui-title-30 [Happy] first title saved with Enter: the pencil keeps focus after the title prop arrives", async () => {
    const { user, spy, rerender } = await setup(null)
    await user.click(pencil())
    await user.type(field(), " 편집{Enter}")
    await waitFor(() => expect(spy).toHaveBeenCalledWith(1, "IMG_0612 편집"))
    await waitFor(() => expect(screen.queryByRole("textbox")).not.toBeInTheDocument())
    // 저장 뒤 재조회된 제목이 prop 으로 도착
    rerender(1, "IMG_0612 편집")
    expect(pencil()).toHaveFocus()
  })

  it("T-ui-title-31 [Happy] title cleared with Enter: the pencil keeps focus after the title prop is cleared", async () => {
    const { user, spy, rerender } = await setup("제주 일출")
    await user.click(pencil())
    await user.clear(field())
    await user.keyboard("{Enter}")
    await waitFor(() => expect(spy).toHaveBeenCalledWith(1, null))
    await waitFor(() => expect(screen.queryByRole("textbox")).not.toBeInTheDocument())
    rerender(1, null)
    expect(pencil()).toHaveFocus()
  })
})
