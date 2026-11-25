// Categories: Happy (05 07 09 10) / Boundary (03 08) / Validation (04) / External (06 11)
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { openFolderSettings } from "@/test/folderDialog"
import { openSettingsPage } from "@/test/settings"

const PATH = "D:\\Photos"
const NEW_PATH = "E:\\Footage"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup() {
  stubLayout()
  const user = userEvent.setup()
  const api = createMockApi({ count: 40 })
  vi.spyOn(api, "pickDirectory").mockResolvedValue(NEW_PATH)
  renderApp(api)
  const settings = await openSettingsPage(user, "관리 폴더")
  await within(settings).findByRole("button", { name: `${PATH} 설정` })
  return { user, api, settings }
}

async function openAddDialog(user: ReturnType<typeof userEvent.setup>, settings: HTMLElement) {
  await user.click(within(settings).getByRole("button", { name: "폴더 추가" }))
  return screen.findByRole("dialog", { name: "관리 폴더 추가" })
}

async function openTypesDialog(user: ReturnType<typeof userEvent.setup>, settings: HTMLElement) {
  return openFolderSettings(user, settings, PATH)
}

describe("add folder dialog (media types)", () => {
  it("T-ui-mtype-03 [Boundary] cancelling the directory pick opens nothing and adds nothing", async () => {
    const { user, api, settings } = await setup()
    vi.spyOn(api, "pickDirectory").mockResolvedValue(null)
    const add = vi.spyOn(api, "addFolder")
    await user.click(within(settings).getByRole("button", { name: "폴더 추가" }))
    await waitFor(() => expect(api.pickDirectory).toHaveBeenCalled())
    expect(screen.queryByRole("dialog", { name: "관리 폴더 추가" })).not.toBeInTheDocument()
    expect(add).not.toHaveBeenCalled()
  })

  it("T-ui-mtype-04 [Validation] unchecking every type shows an inline error and blocks adding", async () => {
    const { user, api, settings } = await setup()
    const add = vi.spyOn(api, "addFolder")
    const dialog = await openAddDialog(user, settings)
    expect(within(dialog).getByText(NEW_PATH)).toBeInTheDocument()
    for (const name of ["동영상", "이미지", "음성"]) {
      expect(within(dialog).getByRole("checkbox", { name })).toBeChecked()
      await user.click(within(dialog).getByRole("checkbox", { name }))
    }
    const alert = within(dialog).getByRole("alert")
    expect(alert).toHaveTextContent("하나 이상 선택하세요.")
    const box = within(dialog).getByRole("checkbox", { name: "동영상" })
    expect(box).toHaveAttribute("aria-invalid", "true")
    expect(box).toHaveAccessibleDescription("하나 이상 선택하세요.")
    expect(within(dialog).getByRole("button", { name: "추가" })).toBeDisabled()
    expect(add).not.toHaveBeenCalled()
  })

  it("T-ui-mtype-05 [Happy] adding with a type unchecked passes the selection and shows the type badges", async () => {
    const { user, api, settings } = await setup()
    const add = vi.spyOn(api, "addFolder")
    const dialog = await openAddDialog(user, settings)
    await user.click(within(dialog).getByRole("checkbox", { name: "이미지" }))
    await user.click(within(dialog).getByRole("button", { name: "추가" }))
    await waitFor(() => expect(add).toHaveBeenCalledWith(NEW_PATH, { image: false, video: true, audio: true }))
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "관리 폴더 추가" })).not.toBeInTheDocument())
    const row = (await within(settings).findByText("Footage")).closest("tr") as HTMLElement
    // 행에는 세 유형이 모두 보이고 포함/제외가 구분된다
    expect(within(row).getByRole("img", { name: "동영상 포함" })).toBeInTheDocument()
    expect(within(row).getByRole("img", { name: "음성 포함" })).toBeInTheDocument()
    expect(within(row).getByRole("img", { name: "이미지 제외" })).toBeInTheDocument()
  })

  it("T-ui-mtype-06 [External] a failing add_folder is shown inline and the dialog stays open", async () => {
    const { user, api, settings } = await setup()
    vi.spyOn(api, "addFolder").mockRejectedValue({ code: "already_registered", message: "x" })
    const dialog = await openAddDialog(user, settings)
    await user.click(within(dialog).getByRole("button", { name: "추가" }))
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("이미 등록되어 있습니다.")
    expect(screen.getByRole("dialog", { name: "관리 폴더 추가" })).toBeInTheDocument()
  })

  it("T-ui-mtype-07 [Happy] the sidebar + button uses the same dialog (all types checked by default)", async () => {
    stubLayout()
    const user = userEvent.setup()
    const api = createMockApi({ count: 40 })
    vi.spyOn(api, "pickDirectory").mockResolvedValue(NEW_PATH)
    const add = vi.spyOn(api, "addFolder")
    renderApp(api)
    const nav = await screen.findByRole("navigation", { name: "라이브러리 탐색" })
    await user.click(within(nav).getByRole("button", { name: "관리 폴더 추가" }))
    const dialog = await screen.findByRole("dialog", { name: "관리 폴더 추가" })
    await user.click(within(dialog).getByRole("button", { name: "추가" }))
    await waitFor(() => expect(add).toHaveBeenCalledWith(NEW_PATH, { image: true, video: true, audio: true }))
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "관리 폴더 추가" })).not.toBeInTheDocument())
    expect((await api.listFolders()).find((x) => x.path === NEW_PATH)?.mediaTypes).toEqual({ image: true, video: true, audio: true })
  })
})

describe("folder media types dialog", () => {
  it("T-ui-mtype-08 [Boundary] shows current counts and hides keep/purge when nothing is excluded", async () => {
    const { user, api, settings } = await setup()
    const counts = await api.countFolderAssetsByType(1)
    const dialog = await openTypesDialog(user, settings)
    expect(await within(dialog).findByText(`(${counts.video.toLocaleString("ko-KR")})`)).toBeInTheDocument()
    expect(within(dialog).queryByRole("radiogroup")).not.toBeInTheDocument()
    expect(within(dialog).getByRole("button", { name: "저장" })).toBeDisabled() // 변경 없음
    // 먼저 하나를 끄면 선택지가 나타나고, 다시 켜면 사라진다
    await user.click(within(dialog).getByRole("checkbox", { name: /이미지/ }))
    expect(within(dialog).getByRole("radiogroup")).toBeInTheDocument()
    await user.click(within(dialog).getByRole("checkbox", { name: /이미지/ }))
    expect(within(dialog).queryByRole("radiogroup")).not.toBeInTheDocument()
  })

  it("T-ui-mtype-09 [Happy] excluding a type defaults to keep and passes it with the new selection", async () => {
    const { user, api, settings } = await setup()
    const set = vi.spyOn(api, "setFolderMediaTypes")
    const counts = await api.countFolderAssetsByType(1)
    const dialog = await openTypesDialog(user, settings)
    await user.click(within(dialog).getByRole("checkbox", { name: /이미지/ }))
    expect(within(dialog).getByRole("radio", { name: /수집된 항목 유지/ })).toBeChecked()
    expect(within(dialog).getByText(new RegExp(`제외되는 유형: 이미지 ${counts.image.toLocaleString("ko-KR")}개`))).toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    await waitFor(() => expect(set).toHaveBeenCalledWith(1, { image: false, video: true, audio: true }, "keep"))
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "관리 폴더 설정" })).not.toBeInTheDocument())
    expect(await api.countFolderAssetsByType(1)).toEqual(counts) // keep: 개수 그대로
  })

  it("T-ui-mtype-10 [Happy] purge shows the safety/re-enable warning and passes mode purge", async () => {
    const { user, api, settings } = await setup()
    const set = vi.spyOn(api, "setFolderMediaTypes")
    const before = await api.countFolderAssetsByType(1)
    const dialog = await openTypesDialog(user, settings)
    await user.click(within(dialog).getByRole("checkbox", { name: /음성/ }))
    expect(within(dialog).getByText(/연결 데이터\(메타정보·썸네일·태그·AI 결과\)가 영구히 삭제됩니다. 원본 파일은 바뀌지 않습니다/)).toBeInTheDocument()
    expect(within(dialog).getByText(/처음부터 다시 스캔하며, 삭제된 태그·메모·AI 분석 결과는 돌아오지 않습니다/)).toBeInTheDocument()
    expect(within(dialog).queryByText(/태그·메모는 보관/)).not.toBeInTheDocument()
    await user.click(within(dialog).getByRole("radio", { name: /수집된 항목 정리/ }))
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    await waitFor(() => expect(set).toHaveBeenCalledWith(1, { image: true, video: true, audio: false }, "purge"))
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "관리 폴더 설정" })).not.toBeInTheDocument())
    expect(await api.countFolderAssetsByType(1)).toEqual({ ...before, audio: 0 })
  })

  it("T-ui-mtype-11 [External] a failing set_folder_media_types is shown inline; unchecking all is blocked", async () => {
    const { user, api, settings } = await setup()
    vi.spyOn(api, "setFolderMediaTypes").mockRejectedValue({ code: "not_found", message: "x" })
    const dialog = await openTypesDialog(user, settings)
    await user.click(within(dialog).getByRole("checkbox", { name: /음성/ }))
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("찾을 수 없습니다")
    expect(screen.getByRole("dialog", { name: "관리 폴더 설정" })).toBeInTheDocument()
    // 모두 끄면 인라인 에러 + 적용 불가
    await user.click(within(dialog).getByRole("checkbox", { name: /동영상/ }))
    await user.click(within(dialog).getByRole("checkbox", { name: /이미지/ }))
    expect(within(dialog).getByRole("alert")).toHaveTextContent("하나 이상 선택하세요.")
    expect(within(dialog).getByRole("button", { name: "저장" })).toBeDisabled()
  })
})
