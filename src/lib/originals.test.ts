// Categories: Happy (T-ui-orig-01) / Boundary (T-ui-orig-02) / Business rule (T-ui-orig-03)
import { describe, expect, it } from "vitest"
import i18n from "@/i18n"
import { ORIGINALS_FAILURE_CODES } from "./ipc-contract"
import { summarizeFailures } from "./originals"

const t = i18n.t.bind(i18n)

describe("originals failure summary", () => {
  it("T-ui-orig-01 [Happy] groups failure codes by count, most frequent first", () => {
    // Arrange
    const failed = [
      { assetId: 1, message: "offline" },
      { assetId: 2, message: "file_missing" },
      { assetId: 3, message: "offline" },
    ]
    // Act
    const text = summarizeFailures(t, failed)
    // Assert
    expect(text).toBe("드라이브 연결 끊김 2건, 파일 없음 1건")
  })

  it("T-ui-orig-02 [Boundary] unknown codes (or legacy free text) fall back to 기타 and never echo the raw message", () => {
    // Arrange
    const failed = [{ assetId: 1, message: "C:\\secret\\path.jpg: access denied" }]
    // Act
    const text = summarizeFailures(t, failed)
    // Assert
    expect(text).toBe("기타 1건")
    expect(text).not.toContain("secret")
  })

  it("T-ui-orig-03 [Business rule] every backend failure code has a Korean reason text (dynamic keys are not seen by the key scanner)", () => {
    // Act + Assert
    for (const code of ORIGINALS_FAILURE_CODES) {
      const key = `originals.reason.${code}`
      expect(i18n.exists(key), key).toBe(true)
    }
    expect(i18n.exists("originals.reason.other")).toBe(true)
  })
})
