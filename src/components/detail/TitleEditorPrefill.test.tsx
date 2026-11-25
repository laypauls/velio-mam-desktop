// Categories: Happy (T-ui-title-08, T-ui-title-10) / Business rule (T-ui-title-09, T-ui-title-11, T-ui-title-12) / Boundary (T-ui-title-13)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { effectiveTitle, splitFileName } from "@/lib/displayName"
import { TitleEditor } from "./TitleEditor"

afterEach(() => vi.restoreAllMocks())

async function setup(title: string | null, fileName = "IMG_0612.jpg") {
  const api = createMockApi({ count: 5 })
  await api.updateSettings({ titleEditEnabled: true })
  const spy = vi.spyOn(api, "setAssetTitle").mockResolvedValue(undefined)
  const user = userEvent.setup()
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <TitleEditor assetId={1} title={title} fileName={fileName} />
      </QueryClientProvider>
    </ApiProvider>
  )
  const enter = async () => {
    await user.click(await screen.findByRole("button", { name: "제목 편집" }))
    return screen.getByRole("textbox", { name: "제목" }) as HTMLInputElement
  }
  const input = await enter()
  return { user, spy, input, enter }
}

describe("title editor prefill (UI only)", () => {
  it("T-ui-title-08 [Happy] with no stored title the input is prefilled with the file name without extension", async () => {
    const { input } = await setup(null)
    expect(input).toHaveValue("IMG_0612")
  })

  it("T-ui-title-09 [Business rule] a stored title is shown as is (no prefill)", async () => {
    const { input } = await setup("제주 일출")
    expect(input).toHaveValue("제주 일출")
  })

  it("T-ui-title-10 [Business rule] saving the unchanged prefill makes no call; editing it saves the new value", async () => {
    // Arrange
    const { user, spy, input, enter } = await setup(null)
    // Act: 변경 없이 Enter (편집 종료)
    await user.click(input)
    await user.keyboard("{Enter}")
    // Assert
    expect(spy).not.toHaveBeenCalled()
    expect(screen.queryByRole("textbox", { name: "제목" })).not.toBeInTheDocument()
    // Act: 수정
    const again = await enter()
    await user.type(again, " 편집{Enter}")
    // Assert
    await waitFor(() => expect(spy).toHaveBeenCalledWith(1, "IMG_0612 편집"))
  })

  it("T-ui-title-11 [Business rule] no clear button without a stored title; Esc reverts to the stem", async () => {
    // Arrange
    const { user, input, enter } = await setup(null)
    // Assert
    expect(screen.queryByRole("button", { name: "제목 지우기 (파일명 사용)" })).not.toBeInTheDocument()
    // Act
    await user.type(input, "x")
    await user.keyboard("{Escape}")
    // Assert
    expect(await enter()).toHaveValue("IMG_0612")
  })

  it("T-ui-title-12 [Business rule] emptying the field with no stored title restores the stem without a call; with a stored title it clears (null) and Esc reverts to the stored title", async () => {
    // Arrange: 저장된 제목 없음
    const a = await setup(null)
    await a.user.clear(a.input)
    await a.user.tab()
    expect(a.spy).not.toHaveBeenCalled()
    expect(await a.enter()).toHaveValue("IMG_0612")
  })

  it("T-ui-title-12b [Business rule] stored title: Esc reverts to it, clear button sends null and shows the stem", async () => {
    // Arrange
    const { user, spy, input, enter } = await setup("제주 일출")
    // Act: Esc
    await user.type(input, "!")
    await user.keyboard("{Escape}")
    expect(await enter()).toHaveValue("제주 일출")
    // Act: 해제
    await user.click(screen.getByRole("button", { name: "제목 지우기 (파일명 사용)" }))
    // Assert
    await waitFor(() => expect(spy).toHaveBeenCalledWith(1, null))
    expect(await enter()).toHaveValue("IMG_0612")
  })
})

describe("splitFileName / effectiveTitle", () => {
  it("T-ui-title-13 [Boundary] only a real extension is stripped; dotfiles, trailing dots and extensionless names stay whole", () => {
    expect(splitFileName("a.b.jpg")).toEqual(["a.b", "JPG"])
    expect(effectiveTitle({ title: null, fileName: ".env" })).toBe(".env")
    expect(effectiveTitle({ title: null, fileName: "name." })).toBe("name.")
    expect(effectiveTitle({ title: null, fileName: "README" })).toBe("README")
    expect(effectiveTitle({ title: "내 제목", fileName: "x.jpg" })).toBe("내 제목")
  })
})
