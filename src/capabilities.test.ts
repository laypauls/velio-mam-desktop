// Categories: Validation (T-ui-fullscreen-16 capability grants what the window wrapper calls)
// 정적 검증 — 상대 경로 import (vitest 의 `/src-tauri` glob 은 watch.ignored 에 걸려 빈 결과가 된다)
import { describe, expect, it } from "vitest"
import capability from "../src-tauri/capabilities/default.json"

describe("main window capability", () => {
  it("T-ui-fullscreen-16 [Validation] grants set-fullscreen and maximize (window wrapper), keeps set-theme, and targets the main window", () => {
    // Assert: isMaximized 는 core:default(core:window:default) 에 포함
    expect(capability.windows).toContain("main")
    expect(capability.permissions).toEqual(expect.arrayContaining(["core:default", "core:window:allow-set-theme", "core:window:allow-set-fullscreen", "core:window:allow-maximize"]))
  })
})
