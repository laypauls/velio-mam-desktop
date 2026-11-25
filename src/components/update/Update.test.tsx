// Categories: Happy + Business rule (T-ui-update-01, -04) / External (T-ui-update-02, -05) / Boundary (T-ui-update-03)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { UiApi } from "@/lib/api"
import { formatBytes } from "@/lib/format"
import type { UpdateStatePayload } from "@/lib/ipc-contract"
import { useUpdateStore } from "@/stores/update"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const UPDATE = { version: "1.2.0", notes: "첫 줄\n둘째 줄", date: "2026-10-20T00:00:00Z" }
const TOAST_TITLE = "새 버전 1.2.0 을(를) 사용할 수 있습니다."

async function untilChecked() {
  await waitFor(() => expect(useUpdateStore.getState().checking).toBe(false))
}

/** 시작 확인(지연 0) → 토스트의 "자세히 보기" 로 업데이트 Dialog 를 연다. */
async function openDialogFromToast(user: ReturnType<typeof userEvent.setup>) {
  await screen.findByText(TOAST_TITLE)
  await user.click(screen.getByRole("button", { name: "자세히 보기" }))
  return screen.findByRole("dialog", { name: "업데이트" })
}

describe("update approval UI (P1-7 / PRD 2.7)", () => {
  it("T-ui-update-01 [Happy/Business rule] available update -> toast + indicator -> dialog; installUpdate runs once, only after 지금 설치", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, update: UPDATE })
    const installSpy = vi.spyOn(api, "installUpdate")
    const user = userEvent.setup()
    renderApp(api, { updateCheckDelayMs: 0 })

    // Act: 알림 단계
    const dialog = await openDialogFromToast(user)

    // Assert: 알림·표시기·Dialog 내용, 아직 설치 안 됨
    expect(screen.getByRole("button", { name: "업데이트 가능: 1.2.0", hidden: true })).toBeTruthy()
    expect(within(dialog).getByText(/0\.1\.0 → 1\.2\.0/)).toBeTruthy()
    expect(within(dialog).getByText(/첫 줄\s+둘째 줄/)).toBeTruthy()
    expect(installSpy).not.toHaveBeenCalled()

    // Act: 승인
    await user.click(within(dialog).getByRole("button", { name: "지금 설치" }))

    // Assert
    expect(await within(dialog).findByText("설치 중… 앱이 다시 시작됩니다.")).toBeTruthy()
    expect(installSpy).toHaveBeenCalledTimes(1)
  })

  it("T-ui-update-02 [External] startup check failure is silent; manual check failure shows inline error in settings", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, updateCheckError: "check_failed" })
    const checkSpy = vi.spyOn(api, "checkUpdate")
    const user = userEvent.setup()
    renderApp(api, { updateCheckDelayMs: 0 })

    // Act: 시작 시 자동 확인
    await waitFor(() => expect(checkSpy).toHaveBeenCalledTimes(1))
    await untilChecked()

    // Assert: 조용함
    expect(screen.queryByRole("alert")).toBeNull()
    expect(screen.queryByText("업데이트를 확인하지 못했습니다. 네트워크 연결을 확인하세요.")).toBeNull()

    // Act: 수동 확인
    const settings = await openSettingsPage(user)
    await user.click(within(settings).getByRole("button", { name: "업데이트 확인" }))

    // Assert
    const alert = await within(settings).findByRole("alert")
    expect(alert.textContent).toBe("업데이트를 확인하지 못했습니다. 네트워크 연결을 확인하세요.")
    expect(checkSpy).toHaveBeenCalledTimes(2)
  })

  it("T-ui-update-03 [Boundary] progress events render downloaded/total, indeterminate when total is null, then installing", async () => {
    // Arrange: 이벤트를 테스트가 직접 발행한다 (타이머 없음)
    stubLayout()
    let emit: (p: UpdateStatePayload) => void = () => undefined
    const base = createMockApi({ count: 300, update: UPDATE })
    const api: UiApi = {
      ...base,
      installUpdate: () => new Promise<void>(() => undefined),
      onUpdateState: async (h) => {
        emit = h
        return () => undefined
      },
    }
    const user = userEvent.setup()
    renderApp(api, { updateCheckDelayMs: 0 })
    const dialog = await openDialogFromToast(user)
    await user.click(within(dialog).getByRole("button", { name: "지금 설치" }))
    await waitFor(() => expect(useUpdateStore.getState().phase).toBe("downloading"))

    // Act + Assert: total 있음
    act(() => emit({ phase: "downloading", downloaded: 5_000_000, total: 10_000_000 }))
    expect(within(dialog).getByText(`${formatBytes(5_000_000)} / ${formatBytes(10_000_000)}`)).toBeTruthy()

    // total 없음 -> 불확정
    act(() => emit({ phase: "downloading", downloaded: 3_000_000, total: null }))
    expect(within(dialog).getByText(`${formatBytes(3_000_000)} 다운로드됨`)).toBeTruthy()

    // 설치 단계 — 버튼 비활성
    act(() => emit({ phase: "installing", downloaded: 10_000_000, total: 10_000_000 }))
    expect(within(dialog).getByText("설치 중… 앱이 다시 시작됩니다.")).toBeTruthy()
    expect((within(dialog).getByRole("button", { name: "나중에" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("T-ui-update-04 [Business rule] 나중에 is remembered for the session: same version is not toasted again", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, update: UPDATE })
    const checkSpy = vi.spyOn(api, "checkUpdate")
    const installSpy = vi.spyOn(api, "installUpdate")
    const user = userEvent.setup()
    renderApp(api, { updateCheckDelayMs: 0 })
    const dialog = await openDialogFromToast(user)

    // Act: 나중에 -> 수동 재확인
    await user.click(within(dialog).getByRole("button", { name: "나중에" }))
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "업데이트" })).toBeNull())
    const settings = await openSettingsPage(user)
    await user.click(within(settings).getByRole("button", { name: "업데이트 확인" }))
    await waitFor(() => expect(checkSpy).toHaveBeenCalledTimes(2))
    await untilChecked()

    // Assert: 설치 없음, 표시기는 유지, 설정 탭에서 다시 열 수 있음
    expect(installSpy).not.toHaveBeenCalled()
    expect(screen.getByRole("button", { name: "업데이트 가능: 1.2.0", hidden: true })).toBeTruthy()
    expect(useUpdateStore.getState().dismissedVersion).toBe("1.2.0")
    expect(useUpdateStore.getState().notifiedVersion).toBe("1.2.0")
    expect(within(settings).getByRole("button", { name: "새 버전 1.2.0 보기" })).toBeTruthy()
  })

  it("T-ui-update-05 [External] install failure shows localized inline error and retry calls installUpdate again", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, update: UPDATE, updateInstallError: "install_failed" })
    const installSpy = vi.spyOn(api, "installUpdate")
    const user = userEvent.setup()
    renderApp(api, { updateCheckDelayMs: 0 })
    const dialog = await openDialogFromToast(user)

    // Act
    await user.click(within(dialog).getByRole("button", { name: "지금 설치" }))

    // Assert
    expect((await within(dialog).findByRole("alert")).textContent).toBe("설치에 실패했습니다.")
    await user.click(within(dialog).getByRole("button", { name: "다시 시도" }))
    await waitFor(() => expect(installSpy).toHaveBeenCalledTimes(2))
  })
})
