// Categories: Happy (T-ui-subtitle-kind-01 every kind resolves to an existing label) / Business rule (T-ui-subtitle-kind-02 stt shows as 자막)
import { describe, expect, it } from "vitest"
import i18n from "@/i18n"
import type { AnalysisKind } from "@/lib/ipc-contract-ai"
import { analysisKindKey } from "./labels"

const ALL_KINDS: Record<AnalysisKind, true> = { describe: true, keywords: true, ocr: true, stt: true, summary: true, embedding: true }

describe("analysisKindKey", () => {
  it.each(Object.keys(ALL_KINDS) as AnalysisKind[])("T-ui-subtitle-kind-01 [Happy] kind %s maps to a key that exists in ko.json (no raw key shown)", (kind) => {
    // Arrange
    const key = analysisKindKey(kind)
    // Act
    const text = i18n.t(key)
    // Assert
    expect(i18n.exists(key)).toBe(true)
    expect(text).not.toBe(key)
    expect(text).not.toMatch(/^ai\.kind\./)
  })

  it("T-ui-subtitle-kind-02 [Business rule] the stt wire value is shown as 자막", () => {
    // Arrange / Act / Assert
    expect(i18n.t(analysisKindKey("stt"))).toBe("자막")
  })
})
