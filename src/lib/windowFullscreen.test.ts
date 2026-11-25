// Categories: Happy (T-ui-fullscreen-20 enter/exit) / Business rule (T-ui-fullscreen-21 maximized restore) / External failure (T-ui-fullscreen-22 reject keeps queue alive) / Concurrency (T-ui-fullscreen-23 serialized)
import { beforeEach, describe, expect, it, vi } from "vitest"

const win = vi.hoisted(() => ({
  calls: [] as string[],
  maximized: false,
  setFullscreen: vi.fn(),
  isMaximized: vi.fn(),
  maximize: vi.fn(),
}))
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => win }))

import { setWindowFullscreen } from "./windowFullscreen"

beforeEach(() => {
  win.calls.length = 0
  win.maximized = false
  win.setFullscreen.mockReset().mockImplementation(async (on: boolean) => void win.calls.push(`fullscreen:${on}`))
  win.isMaximized.mockReset().mockImplementation(async () => win.maximized)
  win.maximize.mockReset().mockImplementation(async () => {
    win.calls.push("maximize")
    win.maximized = true
  })
})

describe("setWindowFullscreen", () => {
  it("T-ui-fullscreen-20 [Happy] enter and exit call the window with the exact Tauri API; a normal window is not maximized afterwards", async () => {
    // Act
    await setWindowFullscreen(true)
    await setWindowFullscreen(false)
    // Assert
    expect(win.calls).toEqual(["fullscreen:true", "fullscreen:false"])
    expect(win.maximize).not.toHaveBeenCalled()
  })

  it("T-ui-fullscreen-21 [Business rule] a window that was maximized before is maximized again after leaving fullscreen (only when fullscreen dropped it)", async () => {
    // Arrange
    win.maximized = true
    // Act
    await setWindowFullscreen(true)
    win.maximized = false // 전체화면 해제가 최대화를 풀었다
    await setWindowFullscreen(false)
    // Assert
    expect(win.calls).toEqual(["fullscreen:true", "fullscreen:false", "maximize"])
    // Act: 이번에는 해제 뒤에도 최대화가 유지된 경우 - 다시 호출하지 않는다
    win.calls.length = 0
    win.maximized = true
    await setWindowFullscreen(true)
    await setWindowFullscreen(false)
    expect(win.calls).toEqual(["fullscreen:true", "fullscreen:false"])
  })

  it("T-ui-fullscreen-22 [External failure] a rejected call rejects to the caller but does not break later calls", async () => {
    // Arrange
    win.setFullscreen.mockRejectedValueOnce(new Error("denied"))
    // Act + Assert
    await expect(setWindowFullscreen(true)).rejects.toThrow("denied")
    await expect(setWindowFullscreen(true)).resolves.toBeUndefined()
    expect(win.calls).toEqual(["fullscreen:true"])
  })

  it("T-ui-fullscreen-23 [Concurrency] overlapping toggles run in order", async () => {
    // Arrange: 첫 호출이 느리다
    let release = () => {}
    win.setFullscreen.mockImplementationOnce(
      (on: boolean) =>
        new Promise<void>((resolve) => {
          release = () => {
            win.calls.push(`fullscreen:${on}`)
            resolve()
          }
        })
    )
    // Act
    const a = setWindowFullscreen(true)
    const b = setWindowFullscreen(false)
    await new Promise((r) => setTimeout(r, 0))
    expect(win.calls).toEqual([])
    release()
    await Promise.all([a, b])
    // Assert
    expect(win.calls).toEqual(["fullscreen:true", "fullscreen:false"])
  })
})
