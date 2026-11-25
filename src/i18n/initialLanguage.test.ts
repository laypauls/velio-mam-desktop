// Categories: Happy (default en, remembered ko) / Validation (unknown stored value) / External (localStorage throws)
// `@/i18n` 은 import 시점에 초기 언어를 정하므로 모듈 캐시를 비우고 매번 새로 import 한다.
import { afterEach, describe, expect, it, vi } from "vitest"

const STORAGE_KEY = "mam.uiLanguage"

async function freshI18n() {
  vi.resetModules()
  return (await import("@/i18n")).default
}

afterEach(() => {
  vi.restoreAllMocks()
  window.localStorage.setItem(STORAGE_KEY, "ko") // 테스트 환경 기본(src/test/setup.ts)
})

describe("initial UI language", () => {
  it("T-ui-i18n-10 [Happy] 저장된 언어가 없으면 영어로 시작하고 <html lang> 도 en 이다", async () => {
    // Arrange
    window.localStorage.removeItem(STORAGE_KEY)
    // Act
    const i18n = await freshI18n()
    // Assert
    expect(i18n.language).toBe("en")
    expect(i18n.t("common.cancel")).toBe("Cancel")
    expect(document.documentElement.lang).toBe("en")
  })

  it("T-ui-i18n-11 [Happy] localStorage 에 ko 가 기억돼 있으면 한국어로 시작한다", async () => {
    // Arrange
    window.localStorage.setItem(STORAGE_KEY, "ko")
    // Act
    const i18n = await freshI18n()
    // Assert
    expect(i18n.language).toBe("ko")
    expect(document.documentElement.lang).toBe("ko")
  })

  it("T-ui-i18n-12 [Validation] 내장 언어가 아닌 값(ja, 빈 문자열)은 무시하고 영어로 시작한다", async () => {
    for (const bad of ["ja", "", "KO", "ko-KR"]) {
      // Arrange
      window.localStorage.setItem(STORAGE_KEY, bad)
      // Act
      const i18n = await freshI18n()
      // Assert
      expect(i18n.language, `stored=${JSON.stringify(bad)}`).toBe("en")
    }
  })

  it("T-ui-i18n-13 [External] localStorage 읽기가 throw 해도 영어로 시작한다", async () => {
    // Arrange
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("denied")
    })
    // Act
    const i18n = await freshI18n()
    // Assert
    expect(i18n.language).toBe("en")
  })

  it("T-ui-i18n-14 [Business] 영어가 폴백이라 누락 키는 한국어가 아니라 영어로 나온다", async () => {
    // Arrange
    window.localStorage.setItem(STORAGE_KEY, "ko")
    const i18n = await freshI18n()
    i18n.removeResourceBundle("ko", "translation")
    // Act
    const text = i18n.t("common.cancel")
    // Assert
    expect(text).toBe("Cancel")
  })
})
