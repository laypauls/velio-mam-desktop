// Categories: Happy / Boundary (missing key, _meta) / External (read failure) / Business rule (built-in overlay merge, en fallback, remembered language)
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import i18n from "@/i18n"
import ko from "@/i18n/locales/ko.json"
import en from "../../src-tauri/locales/en.json"
import type { LocaleResource } from "@/lib/ipc-contract"
import { applyLanguage, currentLocale } from "./applyLanguage"

const KO_CANCEL = ko.common.cancel
const EN_CANCEL = en.common.cancel
const { _meta: _ignored, ...EN_RESOURCE } = en
void _ignored
const STORAGE_KEY = "mam.uiLanguage"

beforeEach(async () => {
  await i18n.changeLanguage("ko")
})

afterEach(async () => {
  await i18n.changeLanguage("ko")
  for (const code of ["ja", "fr"]) i18n.removeResourceBundle(code, "translation")
  i18n.removeResourceBundle("ko", "translation")
  i18n.addResourceBundle("ko", "translation", ko, true, true)
  i18n.removeResourceBundle("en", "translation")
  i18n.addResourceBundle("en", "translation", EN_RESOURCE, true, true)
  window.localStorage.setItem(STORAGE_KEY, "ko")
  document.documentElement.lang = "ko"
})

const reader = (data: LocaleResource) => vi.fn(async () => data)
const failing = () => vi.fn().mockRejectedValue({ code: "not_found", message: "locale not found" })

describe("applyLanguage", () => {
  it("T-ui-locale-01 [Happy] 외부 언어 파일을 리소스로 추가하고 전환하며 <html lang> 을 맞춘다", async () => {
    // Arrange
    const read = reader({ common: { cancel: "キャンセル" } })
    // Act
    const ok = await applyLanguage("ja", read)
    // Assert
    expect(ok).toBe(true)
    expect(read).toHaveBeenCalledWith("ja")
    expect(i18n.language).toBe("ja")
    expect(i18n.t("common.cancel")).toBe("キャンセル")
    expect(document.documentElement.lang).toBe("ja")
    expect(currentLocale()).toBe("ja")
  })

  it("T-ui-locale-02 [Boundary] 언어 파일에 없는 키는 영어로 폴백한다", async () => {
    // Arrange
    const read = reader({ common: { cancel: "キャンセル" } })
    // Act
    await applyLanguage("ja", read)
    // Assert
    expect(i18n.t("common.cancel")).toBe("キャンセル")
    expect(i18n.t("settings.general.language.label")).toBe(en.settings.general.language.label)
    expect(i18n.t("settings.general.language.label")).not.toBe(ko.settings.general.language.label)
  })

  it("T-ui-locale-03 [External] 읽기 실패면 false 를 돌려주고 영어로 되돌린다 (던지지 않음)", async () => {
    // Arrange
    const read = failing()
    // Act
    const ok = await applyLanguage("fr", read)
    // Assert
    expect(ok).toBe(false)
    expect(i18n.language).toBe("en")
    expect(i18n.t("common.cancel")).toBe(EN_CANCEL)
    expect(document.documentElement.lang).toBe("en")
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe("en")
  })

  it("T-ui-locale-04 [External] 이미 다른 언어를 쓰는 중 읽기에 실패하면 영어로 되돌린다", async () => {
    // Arrange
    await applyLanguage("fr", reader({ common: { cancel: "Annuler" } }))
    // Act
    const ok = await applyLanguage("ja", vi.fn().mockRejectedValue({ code: "io", message: "x" }))
    // Assert
    expect(ok).toBe(false)
    expect(i18n.language).toBe("en")
  })

  it("T-ui-locale-05 [Validation] 객체가 아닌 내용은 거부하고 영어로 되돌린다", async () => {
    // Arrange
    const read = vi.fn().mockResolvedValue(["not", "an", "object"])
    // Act
    const ok = await applyLanguage("ja", read)
    // Assert
    expect(ok).toBe(false)
    expect(i18n.language).toBe("en")
  })

  it("T-ui-locale-06 [Boundary] `_meta` 는 번역 리소스에 들어가지 않는다", async () => {
    // Arrange
    const read = reader({ _meta: { name: "日本語" }, common: { cancel: "キャンセル" } })
    // Act
    await applyLanguage("ja", read)
    // Assert
    expect(i18n.getResourceBundle("ja", "translation")).toEqual({ common: { cancel: "キャンセル" } })
  })

  it("T-ui-locale-07 [Business] ko 덮어쓰기 파일은 내장 한국어 위에 키 단위로 병합된다", async () => {
    // Arrange
    const read = reader({ _meta: { name: "한국어" }, common: { cancel: "그만두기" } })
    // Act
    const ok = await applyLanguage("ko", read)
    // Assert
    expect(ok).toBe(true)
    expect(i18n.t("common.cancel")).toBe("그만두기")
    expect(i18n.t("settings.general.language.label")).toBe(ko.settings.general.language.label)
    expect(i18n.getResourceBundle("ko", "translation")).not.toHaveProperty("_meta")
  })

  it("T-ui-locale-08 [External] ko 덮어쓰기 파일이 없어도(읽기 실패) 내장 한국어로 성공 처리한다", async () => {
    // Arrange
    const read = failing()
    // Act
    const ok = await applyLanguage("ko", read)
    // Assert
    expect(ok).toBe(true)
    expect(i18n.language).toBe("ko")
    expect(i18n.t("common.cancel")).toBe(KO_CANCEL)
  })

  it("T-ui-locale-09 [Happy] 다른 언어에서 ko 로 돌아오면 한국어가 적용되고 서식 로케일은 ko-KR 이다", async () => {
    // Arrange
    await applyLanguage("ja", reader({ common: { cancel: "キャンセル" } }))
    // Act
    await applyLanguage("ko", failing())
    // Assert
    expect(i18n.t("common.cancel")).toBe(KO_CANCEL)
    expect(currentLocale()).toBe("ko-KR")
    expect(document.documentElement.lang).toBe("ko")
  })

  it("T-ui-locale-30 [Happy] en 은 내장 영어로 적용되고(파일 없이도 성공) 서식 로케일은 en-US 이다", async () => {
    // Arrange
    const read = failing()
    // Act
    const ok = await applyLanguage("en", read)
    // Assert
    expect(ok).toBe(true)
    expect(i18n.language).toBe("en")
    expect(i18n.t("common.cancel")).toBe(EN_CANCEL)
    expect(currentLocale()).toBe("en-US")
    expect(document.documentElement.lang).toBe("en")
  })

  it("T-ui-locale-31 [Business] 폴더의 en.json 은 내장 영어 위에 키 단위로 덮어쓰고 나머지 키는 내장 그대로다", async () => {
    // Arrange
    const read = reader({ _meta: { name: "English" }, common: { cancel: "Never mind" } })
    // Act
    const ok = await applyLanguage("en", read)
    // Assert
    expect(ok).toBe(true)
    expect(i18n.t("common.cancel")).toBe("Never mind")
    expect(i18n.t("settings.general.language.label")).toBe(en.settings.general.language.label)
    expect(i18n.getResourceBundle("en", "translation")).not.toHaveProperty("_meta")
  })

  it("T-ui-locale-32 [Business] 내장 언어를 적용하면 localStorage 에 기억하고, 폴더 언어팩이면 지운다", async () => {
    // Arrange
    window.localStorage.removeItem(STORAGE_KEY)
    // Act + Assert
    await applyLanguage("en", failing())
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe("en")
    await applyLanguage("ko", failing())
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe("ko")
    await applyLanguage("ja", reader({ common: { cancel: "キャンセル" } }))
    expect(window.localStorage.getItem(STORAGE_KEY)).toBeNull()
  })

  it("T-ui-locale-33 [External] localStorage 쓰기가 throw 해도 언어 적용은 성공한다", async () => {
    // Arrange
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("denied")
    })
    // Act
    const ok = await applyLanguage("en", failing())
    // Assert
    expect(ok).toBe(true)
    expect(i18n.language).toBe("en")
    vi.restoreAllMocks()
  })
})
