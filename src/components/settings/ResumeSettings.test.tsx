// Categories: Happy (toggle, min length, folder / all reset) / Business rule (disabled at 0, hidden folders listed, cancel keeps data, no password) / External (reset failure inline) / Validation (Select sentinel-free)
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import type { UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

vi.mock("sonner", async (importOriginal) => ({ ...(await importOriginal<typeof import("sonner")>()), toast: { error: vi.fn(), success: vi.fn(), info: vi.fn(), warning: vi.fn() } }))

beforeEach(() => {
  // Radix Select 가 jsdom 에 없는 API 를 쓴다
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  vi.clearAllMocks()
})

const STATS = {
  folders: [
    { folderId: 1, count: 128, enabled: true },
    { folderId: 2, count: 0, enabled: false },
    { folderId: 3, count: 12, enabled: true },
  ],
  total: 142,
}

async function openResume(opts: { stats?: typeof STATS; setup?: (api: UiApi) => void } = {}) {
  stubLayout()
  const api = createMockApi({ count: 50 })
  const stats = vi.spyOn(api, "resumeStats").mockResolvedValue(opts.stats ?? STATS)
  const setEnabled = vi.spyOn(api, "resumeSetFolderEnabled").mockResolvedValue(undefined)
  const clearFolder = vi.spyOn(api, "resumeClearFolder").mockResolvedValue(128)
  const clearAll = vi.spyOn(api, "resumeClearAll").mockResolvedValue(142)
  const update = vi.spyOn(api, "updateSettings")
  opts.setup?.(api)
  const user = userEvent.setup()
  renderApp(api)
  const page = await openSettingsPage(user, "이어보기")
  await within(page).findByRole("region", { name: "이어보기 설정" })
  return { api, stats, setEnabled, clearFolder, clearAll, update, user, page }
}

describe("settings > 이어보기", () => {
  it("T-ui-resume-80 [Happy] the menu item sits right after 플레이어, with an h2 title and description", async () => {
    // Arrange + Act
    const { page } = await openResume()
    // Assert
    const nav = within(screen.getByRole("navigation", { name: "설정 메뉴" }))
    const names = nav.getAllByRole("button").map((b) => b.textContent)
    expect(names.indexOf("이어보기")).toBe(names.indexOf("플레이어") + 1)
    expect(within(page).getByRole("heading", { level: 2, name: "이어보기" })).toBeInTheDocument()
    expect(page).toHaveTextContent("동영상·음성의 마지막 재생 위치를 기억해 이어서 재생합니다.")
  })

  it("T-ui-resume-81 [Happy] the 사용 switch defaults ON and saves { resumeEnabled } OFF then ON", async () => {
    // Arrange
    const { page, update, user } = await openResume()
    const sw = within(page).getByRole("switch", { name: "이어보기 사용" })
    expect(sw).toBeChecked()
    // Act + Assert
    await user.click(sw)
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ resumeEnabled: false }))
    await waitFor(() => expect(within(page).getByRole("switch", { name: "이어보기 사용" })).not.toBeChecked())
    await user.click(within(page).getByRole("switch", { name: "이어보기 사용" }))
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ resumeEnabled: true }))
  })

  it("T-ui-resume-82 [Happy] the min length Select shows 3분 by default and saves the chosen seconds (0 = 제한 없음 is a valid non-empty value)", async () => {
    // Arrange
    const { page, update, user } = await openResume()
    const select = within(page).getByRole("combobox", { name: "최소 콘텐츠 길이" })
    expect(select).toHaveTextContent("3분")
    // Act: 10분
    await user.click(select)
    await user.click(await screen.findByRole("option", { name: "10분" }))
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ resumeMinDurationSec: 600 }))
    // Act: 제한 없음 (0)
    await user.click(within(page).getByRole("combobox", { name: "최소 콘텐츠 길이" }))
    const options = (await screen.findAllByRole("option")).map((o) => o.textContent)
    expect(options).toEqual(["제한 없음", "1분", "3분", "5분", "10분"])
    await user.click(screen.getByRole("option", { name: "제한 없음" }))
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ resumeMinDurationSec: 0 }))
  })

  it("T-ui-resume-83 [Business rule] the folder table lists every managed folder (name + path 2 lines) with counts; reset is disabled at 0 records", async () => {
    // Arrange + Act
    const { page } = await openResume()
    const table = await within(page).findByRole("table", { name: "관리 폴더별 이어보기 기록" })
    // Assert
    const rows = within(table).getAllByRole("row").slice(1)
    expect(rows).toHaveLength(3)
    expect(rows[0]).toHaveTextContent("Photos")
    expect(rows[0]).toHaveTextContent("D:\\Photos")
    await waitFor(() => expect(rows[0]).toHaveTextContent("128"))
    expect(within(rows[0]).getByRole("button", { name: /초기화/ })).toBeEnabled()
    expect(within(rows[1]).getByRole("button", { name: /초기화/ })).toBeDisabled()
    expect(within(rows[2]).getByRole("button", { name: /초기화/ })).toBeEnabled()
  })

  it("T-ui-resume-84 [Business rule] hidden folders are listed too, so their records can be reset", async () => {
    // Arrange
    const { page, user, api } = await openResume()
    await api.setFolderHidden(2, true)
    await user.click(within(screen.getByRole("navigation", { name: "설정 메뉴" })).getByRole("button", { name: "관리 폴더" }))
    await user.click(within(screen.getByRole("navigation", { name: "설정 메뉴" })).getByRole("button", { name: "이어보기" }))
    // Act
    const table = await within(screen.getByRole("main")).findByRole("table", { name: "관리 폴더별 이어보기 기록" })
    // Assert
    expect(within(table).getAllByRole("row").slice(1)).toHaveLength(3)
    expect(within(table).getByText("Videos")).toBeInTheDocument()
    expect(page).toBeInTheDocument()
  })

  it("T-ui-resume-85 [Happy] folder reset: AlertDialog names the folder and count and says originals are unaffected; confirm clears that folder, toasts and refreshes stats", async () => {
    // Arrange
    const { page, user, clearFolder, stats } = await openResume()
    const row = (await within(page).findAllByRole("row"))[1]
    await waitFor(() => expect(row).toHaveTextContent("128"))
    const calls = stats.mock.calls.length
    // Act
    await user.click(within(row).getByRole("button", { name: /초기화/ }))
    const dialog = await screen.findByRole("alertdialog")
    // Assert dialog
    expect(dialog).toHaveTextContent("이어보기 기록을 초기화할까요?")
    expect(dialog).toHaveTextContent("\"Photos\" 의 기록 128건이 삭제됩니다.")
    expect(dialog).toHaveTextContent("원본 파일에는 영향이 없습니다.")
    // Act: 확인
    await user.click(within(dialog).getByRole("button", { name: "초기화" }))
    // Assert
    await waitFor(() => expect(clearFolder).toHaveBeenCalledWith(1))
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("이어보기 기록 128건을 초기화했습니다."))
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull())
    await waitFor(() => expect(stats.mock.calls.length).toBeGreaterThan(calls))
  })

  it("T-ui-resume-86 [Business rule] cancelling the folder reset clears nothing", async () => {
    // Arrange
    const { page, user, clearFolder } = await openResume()
    const row = (await within(page).findAllByRole("row"))[1]
    await waitFor(() => expect(row).toHaveTextContent("128"))
    // Act
    await user.click(within(row).getByRole("button", { name: /초기화/ }))
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "취소" }))
    // Assert
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull())
    expect(clearFolder).not.toHaveBeenCalled()
    expect(toast.success).not.toHaveBeenCalled()
  })

  it("T-ui-resume-87 [Happy] 전체 초기화 shows the total, confirms with the total count, and clears all", async () => {
    // Arrange
    const { page, user, clearAll } = await openResume()
    await waitFor(() => expect(page).toHaveTextContent("총 142건"))
    // Act
    await user.click(within(page).getByRole("button", { name: "전체 초기화" }))
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("모든 이어보기 기록 142건이 삭제됩니다.")
    expect(dialog).toHaveTextContent("원본 파일에는 영향이 없습니다.")
    await user.click(within(dialog).getByRole("button", { name: "초기화" }))
    // Assert
    await waitFor(() => expect(clearAll).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("이어보기 기록 142건을 초기화했습니다."))
  })

  it("T-ui-resume-88 [Boundary] 전체 초기화 is disabled when there are no records at all", async () => {
    // Arrange + Act
    const { page } = await openResume({ stats: { folders: [1, 2, 3].map((folderId) => ({ folderId, count: 0, enabled: true })), total: 0 } })
    // Assert
    await waitFor(() => expect(page).toHaveTextContent("총 0건"))
    expect(within(page).getByRole("button", { name: "전체 초기화" })).toBeDisabled()
  })

  it("T-ui-resume-89 [External] a failing reset keeps the dialog open with an inline error and no success toast", async () => {
    // Arrange
    const { page, user, clearAll } = await openResume()
    clearAll.mockRejectedValueOnce({ code: "internal", message: "boom" })
    await waitFor(() => expect(page).toHaveTextContent("총 142건"))
    // Act
    await user.click(within(page).getByRole("button", { name: "전체 초기화" }))
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "초기화" }))
    // Assert
    expect(await within(screen.getByRole("alertdialog")).findByRole("alert")).toBeInTheDocument()
    expect(toast.success).not.toHaveBeenCalled()
  })

  it("T-ui-resume-90 [External] a failing stats load shows an inline error", async () => {
    // Arrange + Act
    const { page } = await openResume({ setup: (api) => void vi.spyOn(api, "resumeStats").mockRejectedValue({ code: "internal", message: "boom" }) })
    // Assert
    expect(await within(page).findByText("이어보기 기록 수를 불러오지 못했습니다.")).toBeInTheDocument()
  })

  it("T-ui-resume-91 [Happy] the 사용 column switches reflect the stats' enabled flag", async () => {
    // Arrange + Act
    const { page } = await openResume()
    // Assert
    const table = await within(page).findByRole("table", { name: "관리 폴더별 이어보기 기록" })
    expect(within(table).getByRole("columnheader", { name: "사용" })).toBeInTheDocument()
    await waitFor(() => expect(within(table).getByRole("switch", { name: "Videos 이어보기 사용" })).not.toBeChecked())
    expect(within(table).getByRole("switch", { name: "Photos 이어보기 사용" })).toBeChecked()
  })

  it("T-ui-resume-92 [Happy] toggling a folder switch calls resumeSetFolderEnabled with the folder id and the new value, then refreshes stats", async () => {
    // Arrange
    const { page, user, setEnabled, stats } = await openResume()
    const off = await within(page).findByRole("switch", { name: "Videos 이어보기 사용" })
    await waitFor(() => expect(off).toBeEnabled())
    const calls = stats.mock.calls.length
    // Act
    await user.click(off)
    await user.click(within(page).getByRole("switch", { name: "Photos 이어보기 사용" }))
    // Assert
    await waitFor(() => expect(setEnabled).toHaveBeenNthCalledWith(1, 2, true))
    expect(setEnabled).toHaveBeenNthCalledWith(2, 1, false)
    await waitFor(() => expect(stats.mock.calls.length).toBeGreaterThan(calls))
  })

  it("T-ui-resume-93 [Business rule] global 사용 OFF disables every folder switch but keeps their values (gate ON path re-enables)", async () => {
    // Arrange
    const { page, user, setEnabled } = await openResume()
    const photos = await within(page).findByRole("switch", { name: "Photos 이어보기 사용" })
    await waitFor(() => expect(photos).toBeEnabled())
    // Act
    await user.click(within(page).getByRole("switch", { name: "이어보기 사용" }))
    // Assert: disabled, values preserved, no call
    await waitFor(() => expect(photos).toBeDisabled())
    expect(within(page).getByRole("switch", { name: "Videos 이어보기 사용" })).toBeDisabled()
    expect(photos).toBeChecked()
    expect(within(page).getByRole("switch", { name: "Videos 이어보기 사용" })).not.toBeChecked()
    await user.click(photos)
    expect(setEnabled).not.toHaveBeenCalled()
    // Act: back ON
    await user.click(within(page).getByRole("switch", { name: "이어보기 사용" }))
    await waitFor(() => expect(photos).toBeEnabled())
  })

  it("T-ui-resume-94 [Boundary] switches are disabled until stats are loaded", async () => {
    // Arrange
    let release: (v: typeof STATS) => void = () => undefined
    const { page } = await openResume({
      setup: (api) => void vi.spyOn(api, "resumeStats").mockReturnValue(new Promise((resolve) => (release = resolve))),
    })
    // Act + Assert
    const sw = await within(page).findByRole("switch", { name: "Photos 이어보기 사용" })
    expect(sw).toBeDisabled()
    release(STATS)
    await waitFor(() => expect(sw).toBeEnabled())
  })

  it("T-ui-resume-95 [External] a failing toggle shows the error toast", async () => {
    // Arrange
    const { page, user, setEnabled } = await openResume()
    setEnabled.mockRejectedValueOnce({ code: "not_found", message: "folder not found" })
    const sw = await within(page).findByRole("switch", { name: "Photos 이어보기 사용" })
    await waitFor(() => expect(sw).toBeEnabled())
    // Act
    await user.click(sw)
    // Assert
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("폴더 이어보기 설정을 바꾸지 못했습니다."))
  })
})
