import { describe, expect, it } from "vitest"
import { errorKey, isCommandError, isSearchNotReady } from "@/lib/errors"

describe("search-not-ready detection", () => {
  it("T-ui-ipc-43 [Happy] index_not_ready is recognised by code, regardless of message", () => {
    // Arrange
    const err = { code: "index_not_ready", message: "anything" }
    // Act + Assert
    expect(isCommandError(err)).toBe(true)
    expect(isSearchNotReady(err)).toBe(true)
    expect(errorKey(err)).toBe("errors.index_not_ready")
  })

  it("T-ui-ipc-44 [Validation] the old internal + message form no longer counts as not-ready", () => {
    // Arrange
    const legacy = { code: "internal", message: "search index is not ready" }
    // Act + Assert
    expect(isSearchNotReady(legacy)).toBe(false)
    expect(isSearchNotReady(new Error("search index is not ready"))).toBe(false)
    expect(isSearchNotReady(null)).toBe(false)
  })
})

describe("folder relocation errors", () => {
  it("T-ui-ipc-45 [Happy] scan_running is a known command error with its own message key", () => {
    // Arrange
    const err = { code: "scan_running", message: "a scan of this folder is running" }
    // Act + Assert
    expect(isCommandError(err)).toBe(true)
    expect(errorKey(err)).toBe("errors.scan_running")
  })
})
