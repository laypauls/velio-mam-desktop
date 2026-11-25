// Categories: Happy (T-ui-scene-03c Ctrl+Arrow, 03d add) / Business rule (T-ui-scene-03e last-target base, 1s window) / Validation (T-ui-scene-03f ignored contexts, 03g non-video, 03h modifiers + repeat)
import { act, fireEvent, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { auto, mountScenePlayer, sceneCuts, user } from "@/test/scene"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), warning: vi.fn(), info: vi.fn(), success: vi.fn() } }))

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

type Mods = { ctrl?: boolean; alt?: boolean; shift?: boolean; repeat?: boolean }
/** 물리 키 입력 — 한글 IME 상태는 key 만 다른 문자로 보낸다. 반환값 false = preventDefault 됨 */
function press(code: string, key: string, mods: Mods = {}, target: Element = document.body): boolean {
  return fireEvent.keyDown(target, { code, key, ctrlKey: !!mods.ctrl, altKey: !!mods.alt, shiftKey: !!mods.shift, repeat: !!mods.repeat })
}
const next = (mods: Mods = { ctrl: true }) => press("ArrowRight", "ArrowRight", mods)
const prev = (mods: Mods = { ctrl: true }) => press("ArrowLeft", "ArrowLeft", mods)
/** 재생 위치를 정하고 컨트롤 바 상태에 반영한다 */
function at(el: HTMLMediaElement, seconds: number) {
  el.currentTime = seconds
  act(() => void el.dispatchEvent(new Event("timeupdate")))
}

/** 영상 길이 100초, fps 25(반 프레임 20ms), 컷 10s(자동) · 20s(사용자) · 30s(자동) */
async function mount(over: Parameters<typeof mountScenePlayer>[0] = {}) {
  const mounted = await mountScenePlayer({ scene: sceneCuts(1, [auto(10_000), user(20_000), auto(30_000)]), ...over })
  await screen.findByRole("button", { name: /장면전환점 표시/ })
  return mounted
}

describe("scene keys on the player (video)", () => {
  it("T-ui-scene-03c [Happy] Ctrl+ArrowRight / Ctrl+ArrowLeft seek to cut + half frame; the key is consumed (preventDefault)", async () => {
    // Arrange
    const { el } = await mount()
    at(el, 5)
    // Act + Assert: 다음
    expect(next()).toBe(false)
    expect(el.currentTime).toBeCloseTo(10.02, 6)
    // 현재 위치가 컷 + 반 프레임 바로 뒤라고 보고하는 상태에서 이전
    vi.spyOn(Date, "now").mockReturnValue(Date.now() + 5000)
    at(el, 30.5)
    prev()
    expect(el.currentTime).toBeCloseTo(30.02, 6)
  })

  it("T-ui-scene-03e [Business rule] rapid presses use the last target (not the stale reported time) and walk one cut at a time; after 1s the current time is the base again; edges do nothing", async () => {
    // Arrange: 위치 보고가 갱신되지 않는 상황(libVLC 계단 시각) — currentTime 은 5s 에 고정으로 보고
    const { el } = await mount()
    const now = vi.spyOn(Date, "now")
    now.mockReturnValue(1_000_000)
    at(el, 5)
    const seeks: number[] = []
    const spy = vi.spyOn(el, "currentTime", "set").mockImplementation((v) => void seeks.push(v))
    vi.spyOn(el, "currentTime", "get").mockReturnValue(5)
    // Act: 연타
    next()
    now.mockReturnValue(1_000_300)
    next()
    now.mockReturnValue(1_000_600)
    next()
    now.mockReturnValue(1_000_900)
    next() // 마지막 컷 뒤 — 동작 없음
    // Assert
    expect(seeks.map((s) => Math.round(s * 1000))).toEqual([10_020, 20_020, 30_020])
    // Act: 마지막 이동 뒤 1초 넘게 지나면 현재 시각(5s)이 다시 기준
    now.mockReturnValue(1_002_500)
    next()
    expect(seeks.map((s) => Math.round(s * 1000))).toEqual([10_020, 20_020, 30_020, 10_020])
    // 반복(repeat) 입력도 이동한다
    now.mockReturnValue(1_002_700)
    next({ ctrl: true, repeat: true })
    expect(seeks).toHaveLength(5)
    spy.mockRestore()
  })

  it("T-ui-scene-03d [Happy] C adds the current position (rounded integer ms), also with a Hangul IME key; Ctrl+C is not taken", async () => {
    // Arrange
    const { el, scene } = await mount()
    at(el, 12.3456)
    // Act
    expect(press("KeyC", "c")).toBe(false)
    // Assert: 정수 ms
    await vi.waitFor(() => expect(scene.add).toHaveBeenCalledWith(1, 12_346))
    // Act: 한글 입력 상태 — key 는 "ㅊ", code 는 KeyC
    at(el, 40)
    press("KeyC", "ㅊ")
    await vi.waitFor(() => expect(scene.add).toHaveBeenCalledWith(1, 40_000))
    // 복사(Ctrl+C) 는 건드리지 않는다
    scene.add.mockClear()
    expect(press("KeyC", "c", { ctrl: true })).toBe(true)
    expect(scene.add).not.toHaveBeenCalled()
  })

  it("T-ui-scene-03h [Validation] C ignores key repeat; Ctrl+Shift+Arrow, Alt+Arrow, Shift+C and plain arrows are not scene keys", async () => {
    // Arrange
    const { el, scene } = await mount()
    at(el, 5)
    // Act
    press("KeyC", "c", { repeat: true })
    next({ ctrl: true, shift: true })
    next({ ctrl: true, alt: true })
    prev({ alt: true })
    press("KeyC", "C", { shift: true })
    // Assert
    expect(scene.add).not.toHaveBeenCalled()
    expect(el.currentTime).toBe(5)
    // 일반 ←/→ 는 기존 5초 이동 그대로 (회귀)
    press("ArrowRight", "ArrowRight")
    expect(el.currentTime).toBe(10)
  })

  it("T-ui-scene-03f [Validation] ignored while typing in an input, and while a dialog / menu is open", async () => {
    // Arrange
    const { el, scene } = await mount()
    at(el, 5)
    const input = screen.getByLabelText("아무 입력")
    // Act 1: 입력 중
    input.focus()
    press("KeyC", "c", {}, input)
    fireEvent.keyDown(input, { code: "ArrowRight", key: "ArrowRight", ctrlKey: true })
    // Assert 1
    expect(scene.add).not.toHaveBeenCalled()
    expect(el.currentTime).toBe(5)
    // Act 2: 열린 Dialog
    const dialog = document.createElement("div")
    dialog.setAttribute("role", "dialog")
    document.body.appendChild(dialog)
    press("KeyC", "c")
    next()
    dialog.remove()
    // Assert 2
    expect(scene.add).not.toHaveBeenCalled()
    expect(el.currentTime).toBe(5)
  })

  it("T-ui-scene-03g [Validation] audio content has no scene UI and the keys do nothing (nothing is even fetched)", async () => {
    // Arrange
    const { el, scene } = await mountScenePlayer({ detail: { mediaType: "audio", fileName: "voice.mp3" } })
    at(el, 5)
    // Act
    press("KeyC", "c")
    next()
    // Assert
    expect(scene.add).not.toHaveBeenCalled()
    expect(scene.get).not.toHaveBeenCalled()
    expect(el.currentTime).toBe(5)
    expect(screen.queryByRole("button", { name: /장면/ })).toBeNull()
  })
})
