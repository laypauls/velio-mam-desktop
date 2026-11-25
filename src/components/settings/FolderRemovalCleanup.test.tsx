// Categories: Happy (T-ui-vcl-01 04 06 07) / External (02 backup_failed, 03 impact failure, 09 forget failure) / Business rule (05 progress, 08 ignored-only, 10 folders)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { FolderRemovalProgress, KnownVolume } from "@/lib/ipc-contract"
import { renderApp, stubLayout } from "@/test/layout"
import { openFolderSettings } from "@/test/folderDialog"
import { openSettingsPage } from "@/test/settings"

vi.mock("sonner", async (orig) => ({ ...(await orig<typeof import("sonner")>()), toast: { error: vi.fn(), success: vi.fn() } }))

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  vi.clearAllMocks()
})

const IMPACT = { mediaCount: 1204, tagLinks: 120, notes: 8, collectionItems: 34, aiResults: 980 }
const CAMERA: KnownVolume = {
  volumeKey: "vol-cam", label: "CAMERA", kind: "removable", lastMountPoint: "G:\\", availability: "offline",
  lastSeenAt: Date.UTC(2026, 9, 2, 3), folderCount: 0, ignored: false,
}
const USB: KnownVolume = { ...CAMERA, volumeKey: "vol-usb", label: "SanDisk USB", lastMountPoint: "E:\\", availability: "online", folderCount: 1 }

async function setup(opts: Parameters<typeof createMockApi>[0] = {}) {
  stubLayout()
  const user = userEvent.setup()
  const api = createMockApi({ count: 30, ...opts })
  let push: (p: FolderRemovalProgress) => void = () => undefined
  vi.spyOn(api, "onFolderRemovalProgress").mockImplementation(async (h) => {
    push = h
    return () => undefined
  })
  renderApp(api)
  await screen.findByRole("navigation", { name: "라이브러리 탐색" })
  await waitFor(() => expect(api.onFolderRemovalProgress).toHaveBeenCalled())
  const main = await openSettingsPage(user, "관리 폴더")
  return { user, api, main, emit: (p: FolderRemovalProgress) => act(() => push(p)) }
}

async function openRemove(user: ReturnType<typeof userEvent.setup>, main: HTMLElement) {
  const settings = await openFolderSettings(user, main, "D:\\Photos")
  await user.click(within(settings).getByRole("button", { name: "폴더 제거" }))
  return screen.findByRole("alertdialog")
}

describe("folder removal dialog", () => {
  it("T-ui-vcl-01 [Happy] shows the safety line, the impact counts, the no-restore/backup sentence and the relocate/hide hint", async () => {
    // Arrange
    const { user, api, main } = await setup({ volumes: { removalImpact: IMPACT } })
    const impact = vi.spyOn(api, "folderRemovalImpact")
    // Act
    const dlg = await openRemove(user, main)
    // Assert
    expect(await within(dlg).findByText("미디어 1,204개의 목록·메타정보·썸네일·미리보기")).toBeInTheDocument()
    expect(within(dlg).getByText("직접 입력한 정보: 태그 120 · 메모 8 · 컬렉션 소속 34")).toBeInTheDocument()
    expect(within(dlg).getByText("AI 분석 결과 980개")).toBeInTheDocument()
    expect(within(dlg).getByText("원본 파일은 지워지지 않습니다.")).toBeInTheDocument()
    expect(within(dlg).getByText(/처음부터 다시 스캔하며, 위 정보는 돌아오지 않습니다. 제거 직전에 데이터베이스를 자동 백업합니다/)).toBeInTheDocument()
    expect(within(dlg).getByText(/\[경로 변경\]을, 목록에서만 감추려면 \[숨기기\]를/)).toBeInTheDocument()
    expect(within(dlg).queryByText(/자산과 태그는 보존/)).not.toBeInTheDocument()
    expect(impact).toHaveBeenCalledTimes(1)
  })

  it("T-ui-vcl-02 [External] backup_failed is shown inline, keeps the dialog open and the button usable for retry", async () => {
    // Arrange
    const { user, api, main } = await setup({ volumes: { removalImpact: IMPACT } })
    const remove = vi.spyOn(api, "removeFolder").mockRejectedValueOnce({ code: "backup_failed", message: "x" })
    const dlg = await openRemove(user, main)
    await within(dlg).findByText("AI 분석 결과 980개")
    // Act
    await user.click(within(dlg).getByRole("button", { name: "폴더 제거" }))
    // Assert
    expect(await within(dlg).findByRole("alert")).toHaveTextContent("백업에 실패해 아무것도 바꾸지 않았습니다")
    expect(screen.getByRole("alertdialog")).toBeInTheDocument()
    expect(within(dlg).getByRole("button", { name: "폴더 제거" })).toBeEnabled()
    expect(toast.success).not.toHaveBeenCalled()
    // 재시도 성공 → 닫힘 + 완료 toast
    await user.click(within(dlg).getByRole("button", { name: "폴더 제거" }))
    await waitFor(() => expect(remove).toHaveBeenCalledTimes(2))
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument())
  })

  it("T-ui-vcl-03 [External] a failing impact lookup shows a generic warning while loading state is announced and removal stays possible", async () => {
    // Arrange
    const { user, api, main } = await setup()
    let fail: (e: unknown) => void = () => undefined
    vi.spyOn(api, "folderRemovalImpact").mockImplementation(() => new Promise((_, reject) => (fail = reject)))
    const remove = vi.spyOn(api, "removeFolder")
    // Act: 로딩 중
    const dlg = await openRemove(user, main)
    // Assert
    expect(within(dlg).getByRole("status")).toHaveTextContent("지워질 항목을 확인하는 중")
    // Act: 실패
    await act(async () => fail({ code: "internal", message: "x" }))
    // Assert
    expect(await within(dlg).findByText(/지워질 항목 수를 확인하지 못했습니다/)).toBeInTheDocument()
    await user.click(within(dlg).getByRole("button", { name: "폴더 제거" }))
    await waitFor(() => expect(remove).toHaveBeenCalledTimes(1))
  })

  it("T-ui-vcl-03b [External] not_found from the impact lookup (already removed/removing) shows inline and disables removal", async () => {
    // Arrange
    const { user, api, main } = await setup()
    vi.spyOn(api, "folderRemovalImpact").mockRejectedValue({ code: "not_found", message: "x" })
    const remove = vi.spyOn(api, "removeFolder")
    // Act
    const dlg = await openRemove(user, main)
    // Assert
    expect(await within(dlg).findByRole("alert")).toHaveTextContent("대상을 찾을 수 없습니다")
    expect(within(dlg).getByRole("button", { name: "폴더 제거" })).toBeDisabled()
    expect(remove).not.toHaveBeenCalled()
  })

  it("T-ui-vcl-04 [Happy] success closes the dialog and toasts that the originals are untouched", async () => {
    // Arrange
    const { user, api, main } = await setup({ volumes: { removalImpact: IMPACT } })
    const remove = vi.spyOn(api, "removeFolder")
    const dlg = await openRemove(user, main)
    await within(dlg).findByText("AI 분석 결과 980개")
    // Act
    await user.click(within(dlg).getByRole("button", { name: "폴더 제거" }))
    // Assert
    await waitFor(() => expect(remove).toHaveBeenCalled())
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("관리 폴더를 제거했습니다. 원본 파일은 그대로 있습니다."))
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument())
    // 제거된 폴더의 설정 다이얼로그도 함께 닫힌다
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "관리 폴더 설정" })).not.toBeInTheDocument())
  })

  it("T-ui-vcl-05 [Business rule] removal progress shows in the status bar, clears when finished, and an error toasts", async () => {
    // Arrange
    const { api, emit } = await setup()
    const refetch = vi.spyOn(api, "listFolders")
    // Act: 진행 중 (두 폴더 합산 750/1500 = 50%)
    await emit({ folderId: 1, done: 500, total: 1000, finished: false, error: null })
    await emit({ folderId: 2, done: 250, total: 500, finished: false, error: null })
    // Assert
    const bar = await screen.findByRole("progressbar")
    expect(bar).toHaveTextContent("폴더 제거 중 50%")
    expect(bar).toHaveAttribute("aria-valuenow", "50")
    // Act: 하나는 정상 종료, 하나는 오류 종료
    await emit({ folderId: 1, done: 1000, total: 1000, finished: true, error: null })
    expect(screen.getByRole("progressbar")).toHaveTextContent("폴더 제거 중 50%")
    await emit({ folderId: 2, done: 300, total: 500, finished: true, error: "io" })
    // Assert
    await waitFor(() => expect(screen.queryByRole("progressbar")).not.toBeInTheDocument())
    expect(toast.error).toHaveBeenCalledTimes(1)
    await waitFor(() => expect(refetch).toHaveBeenCalled())
  })

  it("T-ui-vcl-05b [Boundary] total 0 shows an indeterminate label instead of NaN%", async () => {
    // Arrange
    const { emit } = await setup()
    // Act
    await emit({ folderId: 1, done: 0, total: 0, finished: false, error: null })
    // Assert
    const bar = await screen.findByRole("progressbar")
    expect(bar).toHaveTextContent("폴더 제거 중…")
    expect(bar).not.toHaveAttribute("aria-valuenow")
  })
})

describe("media type purge wording", () => {
  it("T-ui-vcl-06 [Happy] i18n purge texts say connected data is deleted and originals unchanged, with no leftover 'kept' promise", async () => {
    // Arrange
    const ko = (await import("@/i18n/locales/ko.json")).default.settings.folders
    // Act + Assert
    expect(ko.typesPurgeHint).toContain("연결 데이터(메타정보·썸네일·태그·AI 결과)")
    expect(ko.typesPurgeHint).toContain("원본 파일은 바뀌지 않습니다")
    expect(ko.typesPurgeHint).not.toMatch(/보관/)
    expect(ko.typesPurgeWarning).not.toMatch(/다시 연결/)
  })
})

describe("known volumes section", () => {
  it("T-ui-vcl-07 [Happy] one row per disk with state, mount, folder count, last seen and the matching action", async () => {
    // Arrange + Act
    const { main } = await setup({ volumes: { known: [USB, CAMERA], ignored: [{ volumeKey: "vol-old", label: "OLD USB", lastMountPoint: "H:\\", ignoredAt: 1 }] } })
    const list = await within(main).findByRole("list", { name: "기억하는 디스크" })
    const rows = within(list).getAllByRole("listitem")
    // Assert
    expect(rows).toHaveLength(3)
    expect(within(rows[0]).getByText("연결됨")).toBeInTheDocument()
    expect(within(rows[0]).getByText(/E:\\ · 관리 폴더 1개/)).toBeInTheDocument()
    expect(within(rows[0]).getByText("관리 폴더가 있어 기록을 지울 수 없습니다")).toBeInTheDocument()
    expect(within(rows[0]).queryByRole("button")).not.toBeInTheDocument()
    expect(within(rows[1]).getByText("오프라인")).toBeInTheDocument()
    expect(within(rows[1]).getByText(/G:\\ · 마지막 연결 2026\. 10\. 02\./)).toBeInTheDocument()
    expect(within(rows[1]).getByRole("button", { name: "CAMERA 기록 지우기" })).toBeInTheDocument()
    expect(within(rows[2]).getByText("묻지 않음")).toBeInTheDocument()
    expect(within(rows[2]).getByRole("button", { name: "OLD USB 다시 묻기" })).toBeInTheDocument()
    expect(within(rows[2]).queryByRole("button", { name: /기록 지우기/ })).not.toBeInTheDocument()
  })

  it("T-ui-vcl-08 [Happy] forget asks for confirmation with the documented text, then calls forget and the row disappears", async () => {
    // Arrange
    const { user, api, main } = await setup({ volumes: { known: [CAMERA] } })
    const forget = vi.spyOn(api, "forgetVolumeRecord")
    await user.click(await within(main).findByRole("button", { name: "CAMERA 기록 지우기" }))
    const dlg = await screen.findByRole("alertdialog", { name: "CAMERA 디스크 기록을 지울까요?" })
    expect(within(dlg).getByText(/다음에 연결하면 새 디스크로 등록 여부를 묻습니다. 원본 파일과 라이브러리 정보는 바뀌지 않습니다/)).toBeInTheDocument()
    expect(forget).not.toHaveBeenCalled()
    // Act
    await user.click(within(dlg).getByRole("button", { name: "기록 지우기" }))
    // Assert
    await waitFor(() => expect(forget).toHaveBeenCalledWith("vol-cam"))
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument())
    await waitFor(() => expect(within(main).queryByRole("list", { name: "기억하는 디스크" })).not.toBeInTheDocument())
  })

  it("T-ui-vcl-09 [External] volume_has_folders is shown inline in the confirmation and the dialog stays open", async () => {
    // Arrange
    const { user, api, main } = await setup({ volumes: { known: [CAMERA] } })
    vi.spyOn(api, "forgetVolumeRecord").mockRejectedValueOnce({ code: "volume_has_folders", message: "x" })
    await user.click(await within(main).findByRole("button", { name: "CAMERA 기록 지우기" }))
    const dlg = await screen.findByRole("alertdialog")
    // Act
    await user.click(within(dlg).getByRole("button", { name: "기록 지우기" }))
    // Assert
    expect(await within(dlg).findByRole("alert")).toHaveTextContent("관리 폴더가 남아 있어 기록을 지울 수 없습니다")
    expect(screen.getByRole("alertdialog")).toBeInTheDocument()
  })

  it("T-ui-vcl-10 [Business rule] an ignored-only row uses unignore (not forget); the list is refetched after a folder is removed", async () => {
    // Arrange
    const { user, api, main } = await setup({ volumes: { known: [USB], ignored: [{ volumeKey: "vol-old", label: "OLD USB", lastMountPoint: "H:\\", ignoredAt: 1 }] } })
    const unignore = vi.spyOn(api, "unignoreVolume")
    const forget = vi.spyOn(api, "forgetVolumeRecord")
    const list = vi.spyOn(api, "listKnownVolumes")
    await within(main).findByRole("list", { name: "기억하는 디스크" })
    const before = list.mock.calls.length
    // Act
    await user.click(within(main).getByRole("button", { name: "OLD USB 다시 묻기" }))
    // Assert
    await waitFor(() => expect(unignore).toHaveBeenCalledWith("vol-old"))
    expect(forget).not.toHaveBeenCalled()
    await waitFor(() => expect(within(main).queryByText("OLD USB")).not.toBeInTheDocument())
    // Act: 폴더 제거 → 기억하는 디스크 재조회
    const after = list.mock.calls.length
    const dlg = await openRemove(user, main)
    await user.click(within(dlg).getByRole("button", { name: "폴더 제거" }))
    await waitFor(() => expect(list.mock.calls.length).toBeGreaterThan(after))
    expect(after).toBeGreaterThan(before)
  })
})
