// Categories: Validation (T-ui-fullscreen-16 capability grants what the window wrapper calls, T-ui-winctl-20 custom title bar grants) / Business rule (T-ui-winctl-21 native decorations off)
// 정적 검증 — 상대 경로 import (vitest 의 `/src-tauri` glob 은 watch.ignored 에 걸려 빈 결과가 된다)
import { describe, expect, it } from "vitest"
import capability from "../src-tauri/capabilities/default.json"
import tauriConf from "../src-tauri/tauri.conf.json"

describe("main window capability", () => {
  it("T-ui-fullscreen-16 [Validation] grants set-fullscreen and maximize (window wrapper), keeps set-theme, and targets the main window", () => {
    // Assert: isMaximized 는 core:default(core:window:default) 에 포함
    expect(capability.windows).toContain("main")
    expect(capability.permissions).toEqual(expect.arrayContaining(["core:default", "core:window:allow-set-theme", "core:window:allow-set-fullscreen", "core:window:allow-maximize"]))
  })

  it("T-ui-winctl-20 [Validation] grants drag, drag-region double-click, minimize, toggle-maximize, close and is-maximized for the custom title bar", () => {
    expect(capability.permissions).toEqual(
      expect.arrayContaining([
        "core:window:allow-start-dragging",
        "core:window:allow-internal-toggle-maximize",
        "core:window:allow-minimize",
        "core:window:allow-toggle-maximize",
        "core:window:allow-close",
        "core:window:allow-is-maximized",
      ])
    )
  })

  it("T-ui-winctl-21 [Business rule] the main window has no native decorations (custom title bar) and keeps the 960 minimum width", () => {
    const main = tauriConf.app.windows[0]
    expect(main.decorations).toBe(false)
    expect(main.minWidth).toBe(960)
  })
})
