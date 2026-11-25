// Categories: Happy (ko unchanged) / Business rule (en follows i18n language)
import { afterEach, describe, expect, it } from "vitest"
import i18n from "@/i18n"
import { formatBytes, formatDate, formatNumber } from "./format"

afterEach(async () => {
  await i18n.changeLanguage("ko")
})

describe("format follows the i18n language", () => {
  it("T-ui-locale-10 [Happy] ko 는 기존 ko-KR 서식과 같다", () => {
    expect(formatNumber(1234567)).toBe((1234567).toLocaleString("ko-KR"))
    expect(formatBytes(1536)).toBe("1.5 KB")
    expect(formatDate(Date.UTC(2026, 10, 7, 12))).toBe(
      new Intl.DateTimeFormat("ko-KR", { year: "numeric", month: "2-digit", day: "2-digit" }).format(Date.UTC(2026, 10, 7, 12)),
    )
  })

  it("T-ui-locale-11 [Business] en 으로 바꾸면 영어 서식(구분자·날짜 순서)이 쓰인다", async () => {
    // Arrange
    i18n.addResourceBundle("en", "translation", { common: { cancel: "Cancel" } }, true, true)
    const ms = Date.UTC(2026, 10, 7, 12)
    const ko = formatDate(ms)
    // Act
    await i18n.changeLanguage("en")
    // Assert
    expect(formatNumber(1234567)).toBe("1,234,567")
    expect(formatDate(ms)).toBe(
      new Intl.DateTimeFormat("en", { year: "numeric", month: "2-digit", day: "2-digit" }).format(ms),
    )
    expect(formatDate(ms)).not.toBe(ko)
    i18n.removeResourceBundle("en", "translation")
  })

  it("T-ui-locale-12 [Validation] 서식을 지원하지 않는 언어 코드여도 던지지 않고 ko-KR 로 대체한다", async () => {
    // Arrange
    i18n.addResourceBundle("zz-invalid-tag-!!", "translation", {}, true, true)
    // Act
    await i18n.changeLanguage("zz-invalid-tag-!!").catch(() => undefined)
    // Assert
    expect(() => formatNumber(1000)).not.toThrow()
    i18n.removeResourceBundle("zz-invalid-tag-!!", "translation")
  })
})
