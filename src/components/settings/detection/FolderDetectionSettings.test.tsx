// Categories: Happy (01 02 03 06) / Boundary (06 midnight, 08 untouched) / Validation (04 05 06 same) / Business rule (07 warning, 10 unavailable, 11 options) / External (09) / Concurrency-ish partial save (12)
import { fireEvent, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { ChangeDetectorInfo, ManagedFolder } from "@/lib/ipc-contract"
import { renderApp, stubLayout } from "@/test/layout"
import { openFolderSettings } from "@/test/folderDialog"
import { openSettingsPage } from "@/test/settings"

const PATH = "D:\\Photos"
type User = ReturnType<typeof userEvent.setup>

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const DETECTORS: ChangeDetectorInfo[] = [
  { id: "os_events", available: true, reasonKey: null, optionFields: [], experimental: false },
  { id: "scan_only", available: true, reasonKey: null, optionFields: [], experimental: false },
]

async function setup(prepare?: (api: ReturnType<typeof createMockApi>) => Promise<void> | void) {
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

async function openDialog(user: User, settings: HTMLElement) {
  const dialog = await openFolderSettings(user, settings, PATH)
  // 감지기 목록이 도착해야 방식 옵션이 렌더된다
  await waitFor(() => expect(within(dialog).getByRole("combobox", { name: "방식" })).toBeEnabled())
  return dialog
}

async function choose(user: User, dialog: HTMLElement, select: string, option: string | RegExp) {
  await user.click(within(dialog).getByRole("combobox", { name: select }))
  await user.click(await screen.findByRole("option", { name: option }))
}

const save = (user: User, dialog: HTMLElement) => user.click(within(dialog).getByRole("button", { name: "저장" }))

/** `listFolders` 결과의 첫 폴더를 덮어쓴다 (서버가 돌려줄 수 있는 상태를 흉내) */
function overrideFolder(api: ReturnType<typeof createMockApi>, patch: Partial<ManagedFolder>) {
  const orig = api.listFolders.bind(api)
  vi.spyOn(api, "listFolders").mockImplementation(async () => (await orig()).map((f) => (f.folderId === 1 ? { ...f, ...patch } : f)))
}

describe("folder settings dialog — change detection", () => {
  it("T-ui-detect-01 [Happy] opens from the folder row and shows the current behavior (and the fallback reason)", async () => {
    // Arrange
    const { user, settings } = await setup((api) => overrideFolder(api, { watchActive: "scan_only", watchFallbackKey: "start_failed" }))
    // Assert: 폴더 표 행 — 변경 감지 이름 + 폴백 사유 아이콘, 재스캔 칸이 따로 보인다
    expect(await within(settings).findByText("재스캔만")).toBeInTheDocument()
    expect(within(settings).getByRole("img", { name: "이 위치에서는 실시간 감시를 시작하지 못했습니다" })).toBeInTheDocument()
    expect(within(settings).getAllByText(/^6시간마다 \(기본\)/).length).toBeGreaterThan(0)
    // Act
    const dialog = await openDialog(user, settings)
    // Assert
    expect(within(dialog).getByRole("combobox", { name: "방식" })).toHaveTextContent("자동 (권장)")
    expect(within(dialog).getByText("현재 동작: 재스캔만 — 이 위치에서는 실시간 감시를 시작하지 못했습니다")).toBeInTheDocument()
    expect(within(dialog).getByRole("combobox", { name: "일정" })).toHaveTextContent("기본값 사용 (6시간마다)")
  })

  it("T-ui-detect-01b [Business rule] the saved next-rescan line hides while the draft differs and returns when reverted", async () => {
    // Arrange
    const { user, settings } = await setup()
    const dialog = await openDialog(user, settings)
    const next = () => within(dialog).queryByText(/다음 재스캔 예정/)
    expect(next()).toBeInTheDocument()
    // Act / Assert
    await choose(user, dialog, "일정", "요일·시각")
    expect(next()).not.toBeInTheDocument()
    await choose(user, dialog, "일정", /기본값 사용/)
    expect(next()).toBeInTheDocument()
  })

  it("T-ui-detect-02 [Business rule] choosing 자동 sends mode null (never an empty-string select value)", async () => {
    // Arrange
    const { user, api, settings } = await setup(async (a) => void (await a.setFolderWatchMode(1, "scan_only", null)))
    const spy = vi.spyOn(api, "setFolderWatchMode")
    const policySpy = vi.spyOn(api, "setFolderRescanPolicy")
    const dialog = await openDialog(user, settings)
    expect(within(dialog).getByRole("combobox", { name: "방식" })).toHaveTextContent("재스캔만")
    // Act
    await choose(user, dialog, "방식", "자동 (권장)")
    await save(user, dialog)
    // Assert
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(spy).toHaveBeenCalledWith(1, null, null)
    expect(policySpy).not.toHaveBeenCalled()
  })

  it("T-ui-detect-03 [Business rule] choosing 기본값 사용 after a custom policy sends null and keeps the mode untouched", async () => {
    // Arrange
    const { user, api, settings } = await setup(
      async (a) => void (await a.setFolderRescanPolicy(1, { trigger: { kind: "interval", hours: 3 }, window: null, onStart: false })),
    )
    const policySpy = vi.spyOn(api, "setFolderRescanPolicy")
    const modeSpy = vi.spyOn(api, "setFolderWatchMode")
    const dialog = await openDialog(user, settings)
    expect(within(dialog).getByRole("combobox", { name: "일정" })).toHaveTextContent("간격")
    // Act
    await choose(user, dialog, "일정", /기본값 사용/)
    await save(user, dialog)
    // Assert
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(policySpy).toHaveBeenCalledWith(1, null)
    expect(modeSpy).not.toHaveBeenCalled()
  })

  it("T-ui-detect-04 [Validation] hours 0 shows an inline aria-invalid error and does not call the API", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    const spy = vi.spyOn(api, "setFolderRescanPolicy")
    const dialog = await openDialog(user, settings)
    await choose(user, dialog, "일정", "간격")
    const hours = within(dialog).getByRole("spinbutton", { name: "간격" })
    // Act
    await user.clear(hours)
    await user.type(hours, "0")
    // Assert
    expect(hours).toHaveAttribute("aria-invalid", "true")
    const errorId = hours.getAttribute("aria-describedby")
    expect(errorId).not.toBeNull()
    expect(document.getElementById(errorId as string)).toHaveTextContent("1 이상 720 이하의 정수를 입력하세요.")
    // Act: Enter 제출도 막힌다
    await user.type(hours, "{Enter}")
    expect(spy).not.toHaveBeenCalled()
    expect(within(dialog).getByRole("button", { name: "저장" })).toBeDisabled()
  })

  it("T-ui-detect-05 [Validation] an invalid schedule time (25:00) is flagged inline and blocks saving", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    const spy = vi.spyOn(api, "setFolderRescanPolicy")
    const dialog = await openDialog(user, settings)
    await choose(user, dialog, "일정", "요일·시각")
    // Act
    fireEvent.change(within(dialog).getByLabelText("시각"), { target: { value: "25:00" } })
    // Assert
    const at = within(dialog).getByLabelText("시각")
    await waitFor(() => expect(at).toHaveAttribute("aria-invalid", "true"))
    expect(within(dialog).getByText("HH:MM 형식(00:00~23:59)으로 입력하세요")).toBeInTheDocument()
    expect(within(dialog).getByRole("button", { name: "저장" })).toBeDisabled()
    expect(spy).not.toHaveBeenCalled()
  })

  it("T-ui-detect-06 [Boundary] a window across midnight saves; start == end is rejected inline", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    const spy = vi.spyOn(api, "setFolderRescanPolicy")
    const dialog = await openDialog(user, settings)
    await choose(user, dialog, "일정", "간격")
    await user.click(within(dialog).getByRole("checkbox", { name: "이 시간대에만 시작" }))
    // Act: 같은 시각
    fireEvent.change(within(dialog).getByLabelText("시작 시각"), { target: { value: "22:00" } })
    fireEvent.change(within(dialog).getByLabelText("끝 시각"), { target: { value: "22:00" } })
    // Assert
    const error = await within(dialog).findByText("시작과 끝 시각이 같을 수 없습니다")
    for (const name of ["시작 시각", "끝 시각"]) {
      const input = within(dialog).getByLabelText(name)
      expect(input).toHaveAttribute("aria-invalid", "true")
      expect(input).toHaveAttribute("aria-describedby", error.id)
    }
    expect(within(dialog).getByRole("button", { name: "저장" })).toBeDisabled()
    // Act: 자정을 넘김
    fireEvent.change(within(dialog).getByLabelText("끝 시각"), { target: { value: "06:00" } })
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "저장" })).toBeEnabled())
    await save(user, dialog)
    // Assert
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(spy).toHaveBeenCalledWith(1, { trigger: { kind: "interval", hours: 6 }, window: { start: "22:00", end: "06:00" }, onStart: false })
  })

  it("T-ui-detect-07 [Business rule] manual + rescan-only shows the warning in the dialog and the badge on the row", async () => {
    // Arrange
    const { user, settings } = await setup()
    const dialog = await openDialog(user, settings)
    expect(within(dialog).queryByText("직접 재스캔하기 전까지 이 폴더의 변경이 반영되지 않습니다")).not.toBeInTheDocument()
    // Act
    await choose(user, dialog, "방식", "재스캔만")
    await choose(user, dialog, "일정", "수동만")
    // Assert
    expect(within(dialog).getByText("직접 재스캔하기 전까지 이 폴더의 변경이 반영되지 않습니다")).toBeInTheDocument()
    // Act
    await save(user, dialog)
    // Assert: 행 배지
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(await within(settings).findByRole("img", { name: "자동 반영 안 됨" })).toBeInTheDocument()
  })

  it("T-ui-detect-08 [Boundary] an unknown stored trigger kind shows 알 수 없는 정책 and an untouched policy is never sent", async () => {
    // Arrange
    const { user, api, settings } = await setup((a) => overrideFolder(a, { rescanPolicy: { trigger: { kind: "weekly", days: 2 } } }))
    const policySpy = vi.spyOn(api, "setFolderRescanPolicy")
    const modeSpy = vi.spyOn(api, "setFolderWatchMode")
    expect(await within(settings).findByText(/알 수 없는 정책/)).toBeInTheDocument()
    const dialog = await openDialog(user, settings)
    expect(within(dialog).getByRole("combobox", { name: "일정" })).toHaveTextContent("알 수 없는 정책 (기본값으로 동작)")
    // Act: 감지 방식만 바꾼다
    await choose(user, dialog, "방식", "재스캔만")
    await save(user, dialog)
    // Assert
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(modeSpy).toHaveBeenCalledWith(1, "scan_only", null)
    expect(policySpy).not.toHaveBeenCalled()
  })

  it("T-ui-detect-09 [External] a server error is shown inline and the dialog stays open", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    vi.spyOn(api, "setFolderRescanPolicy").mockRejectedValue({ code: "invalid_input", message: "invalid input: rescanPolicy.trigger.hours" })
    const dialog = await openDialog(user, settings)
    await choose(user, dialog, "일정", "수동만")
    // Act
    await save(user, dialog)
    // Assert
    expect(await within(dialog).findByText("입력값이 올바르지 않습니다.")).toBeInTheDocument()
    expect(screen.getByRole("dialog", { name: "관리 폴더 설정" })).toBeInTheDocument()
  })

  it("T-ui-detect-10 [Business rule] an unavailable detector is disabled and shows its reason", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    vi.spyOn(api, "listChangeDetectors").mockResolvedValue([
      DETECTORS[0],
      { id: "scan_only", available: false, reasonKey: "start_failed", optionFields: [], experimental: false },
    ])
    const dialog = await openDialog(user, settings)
    // Act
    await user.click(within(dialog).getByRole("combobox", { name: "방식" }))
    // Assert
    const option = await screen.findByRole("option", { name: /재스캔만 \(사용 불가\)/ })
    expect(option).toHaveAttribute("aria-disabled", "true")
    expect(option).toHaveTextContent("이 위치에서는 실시간 감시를 시작하지 못했습니다")
  })

  it("T-ui-detect-11 [Business rule] a detector-declared integer option renders generically, validates its range, and is sent", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    vi.spyOn(api, "listChangeDetectors").mockResolvedValue([
      { id: "os_events", available: true, reasonKey: null, optionFields: [{ key: "intervalSec", kind: "integer", min: 10, max: 3600, default: 300 }], experimental: false },
      DETECTORS[1],
    ])
    // 이 mock 감지기는 옵션 칸이 없어 옵션을 거부한다 — 호출 인자만 확인하도록 대체한다
    const spy = vi.spyOn(api, "setFolderWatchMode").mockResolvedValue((await api.listFolders())[0])
    const dialog = await openDialog(user, settings)
    await choose(user, dialog, "방식", "실시간 감시")
    // Act: 범위 밖
    const field = within(dialog).getByRole("spinbutton", { name: "intervalSec" })
    expect(field).toHaveValue(300)
    await user.clear(field)
    await user.type(field, "5")
    // Assert
    expect(field).toHaveAttribute("aria-invalid", "true")
    expect(within(dialog).getByRole("button", { name: "저장" })).toBeDisabled()
    // Act: 범위 안
    await user.clear(field)
    await user.type(field, "60")
    await save(user, dialog)
    // Assert
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(spy).toHaveBeenCalledWith(1, "os_events", { intervalSec: 60 })
  })

  it("T-ui-detect-12 [Business rule] a partial save keeps the dialog open; the retry sends only the axis that failed", async () => {
    // Arrange
    const { user, api, settings } = await setup()
    const policySpy = vi.spyOn(api, "setFolderRescanPolicy")
    const modeSpy = vi.spyOn(api, "setFolderWatchMode").mockRejectedValueOnce({ code: "io", message: "x" })
    const dialog = await openDialog(user, settings)
    await choose(user, dialog, "방식", "재스캔만")
    await choose(user, dialog, "일정", "수동만")
    // Act: 첫 저장 — 정책은 성공, 방식은 실패
    await save(user, dialog)
    // Assert
    expect(await within(dialog).findByText("파일 입출력 중 오류가 발생했습니다.")).toBeInTheDocument()
    expect(screen.getByRole("dialog", { name: "관리 폴더 설정" })).toBeInTheDocument()
    expect(policySpy).toHaveBeenCalledTimes(1)
    expect(modeSpy).toHaveBeenCalledTimes(1)
    // Act: 재시도
    await save(user, dialog)
    // Assert
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(policySpy).toHaveBeenCalledTimes(1)
    expect(modeSpy).toHaveBeenCalledTimes(2)
  })
})
