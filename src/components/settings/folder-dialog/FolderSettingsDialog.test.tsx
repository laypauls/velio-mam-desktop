// Categories: Happy (01 01b 08 12) / Boundary (02 03 04 11) / External (05) / Business rule (06 07 09 10)
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { ManagedFolder } from "@/lib/ipc-contract"
import { openFolderSettings } from "@/test/folderDialog"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

const PATH = "D:\\Photos"
const NAS = "\\\\NAS\\Archive"
type User = ReturnType<typeof userEvent.setup>
type Api = ReturnType<typeof createMockApi>

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup(prepare?: (api: Api) => Promise<void> | void) {
  stubLayout()
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  const user = userEvent.setup()
  const api = createMockApi({ count: 10 })
  await prepare?.(api)
  renderApp(api)
  const settings = await openSettingsPage(user, "관리 폴더")
  return { user, api, settings }
}

async function choose(user: User, dialog: HTMLElement, select: string, option: string | RegExp) {
  await user.click(within(dialog).getByRole("combobox", { name: select }))
  await user.click(await screen.findByRole("option", { name: option }))
}

const saveButton = (dialog: HTMLElement) => within(dialog).getByRole("button", { name: "저장" })

/** `listFolders` 결과의 폴더 1번을 덮어쓴다 (서버가 돌려줄 수 있는 상태를 흉내) */
function overrideFolder(api: Api, patch: Partial<ManagedFolder>) {
  const orig = api.listFolders.bind(api)
  vi.spyOn(api, "listFolders").mockImplementation(async () => (await orig()).map((f) => (f.folderId === 1 ? { ...f, ...patch } : f)))
}

describe("folder settings dialog — save", () => {
  it("T-ui-folderdlg-01 [Happy] four axes changed -> each IPC once, one toast, dialog closes", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    const aliasSpy = vi.spyOn(api, "setFolderAlias")
    const typesSpy = vi.spyOn(api, "setFolderMediaTypes")
    const policySpy = vi.spyOn(api, "setFolderRescanPolicy")
    const modeSpy = vi.spyOn(api, "setFolderWatchMode")
    const success = vi.spyOn(toast, "success")
    const dialog = await openFolderSettings(user, settings, PATH)
    await waitFor(() => expect(within(dialog).getByRole("combobox", { name: "방식" })).toBeEnabled())
    // Act
    await user.type(within(dialog).getByRole("textbox", { name: "별칭" }), "새 이름")
    await user.click(within(dialog).getByRole("checkbox", { name: /음성/ }))
    await choose(user, dialog, "방식", "재스캔만")
    await choose(user, dialog, "일정", "수동만")
    await user.click(saveButton(dialog))
    // Assert
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(aliasSpy).toHaveBeenCalledTimes(1)
    expect(aliasSpy).toHaveBeenCalledWith(1, "새 이름")
    expect(typesSpy).toHaveBeenCalledTimes(1)
    expect(typesSpy).toHaveBeenCalledWith(1, { image: true, video: true, audio: false }, "keep")
    expect(policySpy).toHaveBeenCalledTimes(1)
    expect(modeSpy).toHaveBeenCalledTimes(1)
    expect(modeSpy).toHaveBeenCalledWith(1, "scan_only", null)
    // 스캔 대상 변경은 자체 결과 toast 만 띄운다 — 합쳐서 1회
    expect(success).toHaveBeenCalledTimes(1)
  })

  it("T-ui-folderdlg-01b [Happy] changes without a media-type change show the saved toast once and skip the types IPC", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    const typesSpy = vi.spyOn(api, "setFolderMediaTypes")
    const success = vi.spyOn(toast, "success")
    const dialog = await openFolderSettings(user, settings, PATH)
    // Act
    await user.type(within(dialog).getByRole("textbox", { name: "별칭" }), "사진")
    await user.click(saveButton(dialog))
    // Assert
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(typesSpy).not.toHaveBeenCalled()
    expect(success).toHaveBeenCalledTimes(1)
    expect(success).toHaveBeenCalledWith("폴더 설정을 저장했습니다")
  })

  it("T-ui-folderdlg-02 [Boundary] nothing changed -> save is disabled; an edit enables it and reverting disables it again", async () => {
    // Arrange
    const { user, settings } = await setup()
    const dialog = await openFolderSettings(user, settings, PATH)
    const alias = within(dialog).getByRole("textbox", { name: "별칭" })
    // Assert
    expect(saveButton(dialog)).toBeDisabled()
    // Act / Assert
    await user.type(alias, "a")
    expect(saveButton(dialog)).toBeEnabled()
    await user.clear(alias)
    expect(saveButton(dialog)).toBeDisabled()
  })

  it("T-ui-folderdlg-03 [Boundary] 65-character alias shows an inline aria-invalid error and blocks saving", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    const spy = vi.spyOn(api, "setFolderAlias")
    const dialog = await openFolderSettings(user, settings, PATH)
    const alias = within(dialog).getByRole("textbox", { name: "별칭" })
    // Act
    await user.click(alias)
    await user.paste("가".repeat(65))
    // Assert
    const alert = await within(dialog).findByRole("alert")
    expect(alert).toHaveTextContent("64자 이하")
    expect(alias).toHaveAttribute("aria-invalid", "true")
    expect(alias.getAttribute("aria-describedby")).toBe(alert.id)
    expect(saveButton(dialog)).toBeDisabled()
    expect(spy).not.toHaveBeenCalled()
  })

  it("T-ui-folderdlg-04 [Boundary] unchecking every media type shows the inline error and disables saving", async () => {
    // Arrange
    const { user, settings } = await setup()
    const dialog = await openFolderSettings(user, settings, PATH)
    // Act
    for (const name of ["동영상", "이미지", "음성"]) await user.click(within(dialog).getByRole("checkbox", { name: new RegExp(name) }))
    // Assert
    expect(within(dialog).getByRole("alert")).toHaveTextContent("하나 이상 선택하세요.")
    expect(saveButton(dialog)).toBeDisabled()
  })

  it("T-ui-folderdlg-05 [External] a failing media-type save keeps the dialog open with an inline error; the retry does not resend the alias", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    const aliasSpy = vi.spyOn(api, "setFolderAlias")
    const typesSpy = vi.spyOn(api, "setFolderMediaTypes").mockRejectedValueOnce({ code: "not_found", message: "x" })
    const dialog = await openFolderSettings(user, settings, PATH)
    await user.type(within(dialog).getByRole("textbox", { name: "별칭" }), "사진")
    await user.click(within(dialog).getByRole("checkbox", { name: /음성/ }))
    // Act: 첫 저장 — 별칭은 성공, 스캔 대상은 실패
    await user.click(saveButton(dialog))
    // Assert
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("대상을 찾을 수 없습니다.")
    expect(screen.getByRole("dialog", { name: "관리 폴더 설정" })).toBeInTheDocument()
    expect(aliasSpy).toHaveBeenCalledTimes(1)
    expect(typesSpy).toHaveBeenCalledTimes(1)
    // Act: 재시도
    await user.click(saveButton(dialog))
    // Assert
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(aliasSpy).toHaveBeenCalledTimes(1)
    expect(typesSpy).toHaveBeenCalledTimes(2)
  })

  it("T-ui-folderdlg-06 [Business rule] purge is sent only when the purge radio is chosen", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    const typesSpy = vi.spyOn(api, "setFolderMediaTypes")
    const dialog = await openFolderSettings(user, settings, PATH)
    // Act
    await user.click(within(dialog).getByRole("checkbox", { name: /음성/ }))
    await user.click(within(dialog).getByRole("radio", { name: /수집된 항목 정리/ }))
    await user.click(saveButton(dialog))
    // Assert
    await waitFor(() => expect(typesSpy).toHaveBeenCalledWith(1, { image: true, video: true, audio: false }, "purge"))
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
  })
})

describe("folder settings dialog — immediate actions", () => {
  it("T-ui-folderdlg-07 [Business rule] with unsaved edits rescan/relocate/remove are disabled and a hint shows; reverting re-enables them", async () => {
    // Arrange
    const { user, settings } = await setup()
    const dialog = await openFolderSettings(user, settings, PATH)
    const names = ["지금 재스캔", "폴더 변경", "폴더 제거"]
    for (const name of names) expect(within(dialog).getByRole("button", { name })).toBeEnabled()
    expect(within(dialog).queryByText(/저장하거나 취소한 뒤 사용할 수 있습니다/)).not.toBeInTheDocument()
    // Act
    const alias = within(dialog).getByRole("textbox", { name: "별칭" })
    await user.type(alias, "x")
    // Assert
    for (const name of names) expect(within(dialog).getByRole("button", { name })).toBeDisabled()
    expect(within(dialog).getByText(/저장하거나 취소한 뒤 사용할 수 있습니다/)).toBeInTheDocument()
    // Act
    await user.clear(alias)
    // Assert
    for (const name of names) expect(within(dialog).getByRole("button", { name })).toBeEnabled()
  })

  it("T-ui-folderdlg-08 [Happy] rescan now calls the IPC and keeps the dialog open", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    const spy = vi.spyOn(api, "rescanFolder")
    const dialog = await openFolderSettings(user, settings, PATH)
    // Act
    await user.click(within(dialog).getByRole("button", { name: "지금 재스캔" }))
    // Assert
    await waitFor(() => expect(spy).toHaveBeenCalledWith(1))
    expect(screen.getByRole("dialog", { name: "관리 폴더 설정" })).toBeInTheDocument()
  })

  it("T-ui-folderdlg-09 [Business rule] an offline folder cannot be rescanned but can be relocated", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    const spy = vi.spyOn(api, "rescanFolder")
    // Act
    const dialog = await openFolderSettings(user, settings, NAS)
    // Assert
    expect(within(dialog).getByRole("button", { name: "지금 재스캔" })).toBeDisabled()
    expect(within(dialog).getByRole("button", { name: "폴더 변경" })).toBeEnabled()
    expect(spy).not.toHaveBeenCalled()
  })
})

describe("folder table row", () => {
  it("T-ui-folderdlg-10 [Business rule] the table has column headers; excluded types and the no-auto-reflect warning are exposed by label", async () => {
    // Arrange / Act
    const { settings } = await setup((api) =>
      overrideFolder(api, {
        mediaTypes: { image: true, video: false, audio: true },
        watchActive: "scan_only",
        rescanPolicy: { trigger: { kind: "manual" }, window: null, onStart: false },
      }),
    )
    const table = await within(settings).findByRole("table", { name: "관리 폴더" })
    const row = within(table).getByText("Photos").closest("tr") as HTMLElement
    // Assert: 머리글
    for (const name of ["폴더", "대상", "변경 감지", "재스캔", "자산", "작업"]) {
      expect(within(table).getByRole("columnheader", { name })).toBeInTheDocument()
    }
    // Assert: 대상 — 세 유형 모두, 제외는 라벨로 구분
    expect(within(row).getByRole("img", { name: "이미지 포함" })).toBeInTheDocument()
    expect(within(row).getByRole("img", { name: "동영상 제외" })).toBeInTheDocument()
    expect(within(row).getByRole("img", { name: "음성 포함" })).toBeInTheDocument()
    // Assert: 변경 감지·재스캔·경고
    expect(within(row).getByText("재스캔만")).toBeInTheDocument()
    expect(within(row).getByText(/^수동/)).toBeInTheDocument()
    expect(within(row).getByRole("img", { name: "자동 반영 안 됨" })).toBeInTheDocument()
  })

  it("T-ui-folderdlg-10b [Business rule] only exception badges show; an offline network folder shows 감시 안 함, fallback shows a reason icon", async () => {
    // Arrange / Act
    const { settings } = await setup((api) => overrideFolder(api, { watchActive: "scan_only", watchFallbackKey: "start_failed" }))
    const nas = (await within(settings).findAllByText(NAS))[0].closest("tr") as HTMLElement
    const photos = (await within(settings).findByText("Photos")).closest("tr") as HTMLElement
    // Assert: 오프라인은 배지, 네트워크는 텍스트 배지 대신 경로 앞 아이콘(라벨), 감시 안 함은 흐린 글자
    expect(within(nas).queryByText("네트워크")).not.toBeInTheDocument()
    expect(within(nas).getByRole("img", { name: "네트워크 저장 장치" })).toBeInTheDocument()
    expect(within(nas).getByText("오프라인")).toBeInTheDocument()
    expect(within(nas).getByText("감시 안 함")).toBeInTheDocument()
    // Assert: 기본값(내장·온라인·대기)은 배지로 표시하지 않고, 연결 상태는 점의 라벨로만 제공한다
    expect(within(photos).queryByText("내장")).not.toBeInTheDocument()
    expect(within(photos).queryByText("대기")).not.toBeInTheDocument()
    expect(within(photos).queryByText("연결됨")).not.toBeInTheDocument()
    expect(within(photos).getByRole("img", { name: "연결됨" })).toBeInTheDocument()
    expect(within(photos).getByRole("img", { name: "이 위치에서는 실시간 감시를 시작하지 못했습니다" })).toBeInTheDocument()
    expect(within(photos).queryByRole("img", { name: "자동 반영 안 됨" })).not.toBeInTheDocument()
  })

  it("T-ui-folderdlg-11 [Boundary] the alias is the folder label with the full path as tooltip; the asset count is shown", async () => {
    // Arrange / Act
    const { api, settings } = await setup(async (a) => void (await a.setFolderAlias(1, "사진")))
    const row = (await within(settings).findByText("사진")).closest("tr") as HTMLElement
    const count = (await api.listFolders())[0].assetCount
    // Assert
    expect(within(row).getByText("사진")).toHaveAttribute("title", PATH)
    expect(within(row).getByText("\\Photos")).toBeInTheDocument() // 경로 줄은 별칭이 있어도 유지(마지막 폴더는 별도 요소)
    expect(within(row).getByText(count.toLocaleString("ko-KR"))).toBeInTheDocument()
  })

  it("T-ui-folderdlg-12 [Happy] last scan time and scan state are shown in the dialog, not in the row", async () => {
    // Arrange
    const { user, settings } = await setup()
    const row = (await within(settings).findByText("Photos")).closest("tr") as HTMLElement
    // Assert: 행에는 없다
    expect(within(row).queryByText(/마지막 스캔/)).not.toBeInTheDocument()
    // Act
    const dialog = await openFolderSettings(user, settings, PATH)
    // Assert
    expect(within(dialog).getByText(/마지막 스캔: .* · 대기/)).toBeInTheDocument()
  })

  it("T-ui-folderdlg-13 [Boundary] the next rescan time is its own element under the policy; absent when nothing is scheduled", async () => {
    // Arrange / Act
    const { settings } = await setup((api) => overrideFolder(api, { nextRescanAt: Date.UTC(2026, 10, 21, 7, 0) }))
    const withNext = (await within(settings).findByText("Photos")).closest("tr") as HTMLElement
    const nas = (await within(settings).findAllByText(NAS))[0].closest("tr") as HTMLElement
    // Assert: 정책 문구와 별도 요소
    const next = within(withNext).getByText(/^다음 /)
    expect(next.textContent).not.toContain("시간마다")
    expect(within(withNext).getByText(/^6시간마다/)).not.toBe(next)
    // Assert: 예정이 없으면 두 번째 줄이 없다 (수동 정책 + 예정 없음)
    expect(within(nas).queryByText(/^다음 /)).not.toBeInTheDocument()
  })
})
