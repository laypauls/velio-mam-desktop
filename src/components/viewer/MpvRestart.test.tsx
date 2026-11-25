// T-ui-mpv-restart-01..: 워커 충돌 → 자동 재시작 안내 / 재시작 실패 → 다시 시도·외부 열기 (게이트 ON: 이벤트로 켜지는 경로)
// Categories: Happy (-01 restarting notice, -03 retry succeeds) / Business rule (-02 failed notice, -05 external open, -06 clicks isolated, -08 steps)
//             External failure (-04 retry fails) / Boundary (-07 attempt counter, -09 session filter)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { mountMpv } from "@/test/mpv"

vi.mock("sonner", async (importOriginal) => ({
  ...(await importOriginal<typeof import("sonner")>()),
  toast: { error: vi.fn(), warning: vi.fn(), info: vi.fn(), success: vi.fn() },
}))

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const RESTARTING = { state: "restarting", timeMs: 12_000, restart: { attempt: 1, max: 3 } } as const
const FAILED = { state: "restartFailed", timeMs: 12_000, error: "player worker stopped repeatedly", errorCode: "player_worker_crashed", restart: { attempt: 3, max: 3 } } as const

async function playing() {
  const t = await mountMpv({ reply: { state: "playing", timeMs: 12_000 } })
  await t.ready()
  t.sendFrame({ slot: 0, seq: 1 })
  return t
}

describe("MpvPlayer — worker restart notice", () => {
  it("T-ui-mpv-restart-01 [Happy] while the worker restarts a status notice with the attempt shows over the kept last frame; a restored state removes it", async () => {
    // Arrange
    const t = await playing()
    // Act
    t.emitState({ ...RESTARTING })
    // Assert
    expect(await screen.findByRole("status")).toHaveTextContent("플레이어를 다시 시작하는 중… (1/3)")
    expect(t.canvas()).toBeInTheDocument() // 마지막 프레임을 남기는 캔버스는 그대로
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
    // Act: 복원된 상태
    t.emitState({ state: "playing", timeMs: 12_040 })
    // Assert
    expect(screen.queryByText(/플레이어를 다시 시작하는 중/)).not.toBeInTheDocument()
  })

  it("T-ui-mpv-restart-02 [Business rule] restartFailed keeps the stage and shows an alert with [Retry] and [Open externally] — it is not the open-failure notice and does not skip the item", async () => {
    // Arrange + Act
    const t = await playing()
    t.emitState({ ...FAILED })
    // Assert
    const alert = await screen.findByRole("alert")
    expect(alert).toHaveTextContent("플레이어를 시작할 수 없습니다")
    expect(within(alert).getByRole("button", { name: "다시 시도" })).toBeEnabled()
    expect(within(alert).getByRole("button", { name: "외부 프로그램으로 열기" })).toBeInTheDocument()
    expect(t.stage()).not.toContainElement(alert) // role=img 무대의 자식이면 보조 기술에 노출되지 않는다
    expect(t.stage()).toBeInTheDocument()
    expect(t.canvas()).toBeInTheDocument()
    expect(t.close).not.toHaveBeenCalled() // 플레이어를 닫지 않는다 (다시 시도할 수 있어야 한다)
  })

  it("T-ui-mpv-restart-03 [Happy] Retry calls player_restart once (busy while in flight); on success the notice goes and the new session's state is shown", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await playing()
    let finish: () => void = () => undefined
    t.restart.mockImplementation(() => new Promise((r) => (finish = () => r({ sessionId: 2, lengthMs: 120_000, fps: 25, state: "playing", timeMs: 12_000 }))))
    t.emitState({ ...FAILED })
    const retry = await screen.findByRole("button", { name: "다시 시도" })
    // Act
    await user.click(retry)
    // Assert: 진행 중
    expect(t.restart).toHaveBeenCalledTimes(1)
    expect(screen.getByRole("button", { name: "다시 시도" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "다시 시도" })).toHaveAttribute("aria-busy", "true")
    // Act: 성공
    await act(async () => finish())
    // Assert
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument())
    expect(screen.getByRole("button", { name: "일시정지" })).toBeInTheDocument()
    expect(toast.error).not.toHaveBeenCalled()
  })

  it("T-ui-mpv-restart-04 [External failure] a failing Retry shows the error as a toast, keeps the notice and re-enables the button", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await playing()
    t.restart.mockRejectedValue({ code: "player_restart_failed", message: "x" })
    t.emitState({ ...FAILED })
    // Act
    await user.click(await screen.findByRole("button", { name: "다시 시도" }))
    // Assert
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("플레이어를 다시 시작하지 못했습니다. 다시 시도하거나 외부 프로그램으로 열어 주세요."))
    expect(screen.getByRole("alert")).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole("button", { name: "다시 시도" })).toBeEnabled())
  })

  it("T-ui-mpv-restart-05 [Business rule] [Open externally] reuses the external-open behaviour for the current asset", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await playing()
    t.emitState({ ...FAILED })
    // Act
    await user.click(await screen.findByRole("button", { name: "외부 프로그램으로 열기" }))
    // Assert
    expect(t.openOriginal).toHaveBeenCalledWith(1)
  })

  it("T-ui-mpv-restart-06 [Business rule] clicking inside the notice does not toggle play on the stage", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await playing()
    t.emitState({ ...FAILED })
    t.restart.mockResolvedValue({ sessionId: 2, lengthMs: 120_000, fps: 25, state: "paused", timeMs: 0 })
    // Act
    await user.click(await screen.findByRole("button", { name: "다시 시도" }))
    // Assert: 안내 안의 클릭은 무대(재생/일시정지 토글)로 번지지 않는다
    expect(t.restart).toHaveBeenCalledTimes(1)
    expect(t.control).not.toHaveBeenCalled()
    // 무대를 직접 누르면 토글된다 (같은 스텁으로 반증)
    await user.click(t.stage())
    expect(t.control).toHaveBeenCalledWith("toggle")
  })

  it("T-ui-mpv-restart-07 [Boundary] the attempt counter follows the events; a single-attempt policy shows no counter", async () => {
    // Arrange
    const t = await playing()
    // Act + Assert
    t.emitState({ state: "restarting", timeMs: 1, restart: { attempt: 2, max: 3 } })
    expect(screen.getByText(/플레이어를 다시 시작하는 중… \(2\/3\)/)).toBeInTheDocument()
    t.emitState({ state: "restarting", timeMs: 1, restart: { attempt: 1, max: 1 } })
    expect(screen.getByText("플레이어를 다시 시작하는 중…")).toBeInTheDocument()
  })

  it("T-ui-mpv-restart-08 [Business rule] a frame step during the restore fails with player_restarting without breaking the UI (retryable, silent)", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await playing()
    t.emitState({ state: "paused", timeMs: 12_000 })
    t.emitState({ ...RESTARTING, state: "restarting" })
    t.control.mockRejectedValue({ code: "player_restarting", message: "x" })
    // Act
    await user.click(screen.getByRole("button", { name: "앞으로 (정지 시 1프레임)" }))
    // Assert
    expect(t.control).toHaveBeenLastCalledWith("stepForward")
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
    expect(toast.error).not.toHaveBeenCalled()
  })

  it("T-ui-mpv-restart-09 [Boundary] after a restart reply the new session id is the filter: the old session's events are ignored, newer ones accepted", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await playing()
    t.emitState({ ...FAILED })
    await user.click(await screen.findByRole("button", { name: "다시 시도" })) // 응답: sessionId 2, playing
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument())
    // Act + Assert: 세션 1 의 늦은 이벤트는 무시
    t.emitState({ sessionId: 1, state: "paused", timeMs: 99_000 })
    expect(screen.getByRole("button", { name: "일시정지" })).toBeInTheDocument()
    t.emitState({ sessionId: 2, state: "paused", timeMs: 13_000 })
    expect(screen.getByRole("button", { name: "재생" })).toBeInTheDocument()
    expect(screen.getByLabelText("0:13 / 2:00")).toBeInTheDocument()
  })
})
