// Categories: Happy (list, change persists, open folder, refresh) / Validation (error entries) / External (apply & persist failure) / Business rule (two built-ins, folder en/ko hidden, en fallback)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import i18n from "@/i18n"
import { applyLanguage } from "@/i18n/applyLanguage"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { LocaleList, Settings } from "@/lib/ipc-contract"
import { LanguageSection } from "./LanguageSection"

const LIST: LocaleList = {
  dir: "C:\\locales",
  locales: [
    { code: "en", name: "English", error: null },
    { code: "ko", name: "한국어", error: null },
    { code: "fr", name: null, error: null },
    { code: "it", name: null, error: null },
    { code: "ja", name: "日本語", error: "invalid_json" },
    { code: "xx", name: null, error: "too_large" },
  ],
}

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(async () => {
  vi.restoreAllMocks()
  await i18n.changeLanguage("ko")
  i18n.removeResourceBundle("fr", "translation")
})

async function setup(opts: { language?: string; list?: LocaleList } = {}) {
  const api = createMockApi({ count: 10 })
  vi.spyOn(api, "listLocales").mockResolvedValue(opts.list ?? LIST)
  vi.spyOn(api, "readLocale").mockImplementation(async (code) => {
    if (code === "fr") return { _meta: { name: "Français" }, common: { cancel: "Annuler" } }
    throw { code: "io", message: "read failed" }
  })
  const update = vi.spyOn(api, "updateSettings")
  const openFolder = vi.spyOn(api, "openLocaleFolder")
  const settings = { ...(await api.getSettings()), language: opts.language ?? "en" } as Settings
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const user = userEvent.setup()
  render(
    <QueryClientProvider client={qc}>
      <ApiProvider api={api}>
        <LanguageSection settings={settings} />
      </ApiProvider>
    </QueryClientProvider>,
  )
  const select = await screen.findByRole("combobox", { name: "표시 언어 선택" })
  await waitFor(() => expect(api.listLocales).toHaveBeenCalled())
  return { api, update, openFolder, user, select }
}

describe("LanguageSection", () => {
  it("T-ui-locale-20 [Happy] 내장 English(기본)·한국어가 위에 나오고 폴더의 en/ko 는 항목으로 나오지 않는다", async () => {
    // Arrange + Act
    const { user, select } = await setup()
    expect(select).toHaveTextContent("English (default)")
    await user.click(select)
    // Assert
    const options = await screen.findAllByRole("option")
    expect(options.map((o) => o.textContent)).toEqual(["English (default)", "한국어", "fr", "it"])
  })

  it("T-ui-locale-21 [Validation] 오류 파일은 선택할 수 없고 사유가 인라인 경고로 보인다", async () => {
    // Arrange + Act
    await setup()
    // Assert
    const warnings = await screen.findByRole("list", { name: "사용할 수 없는 언어 파일" })
    expect(within(warnings).getByText("ja.json: 올바른 JSON 파일이 아닙니다.")).toBeInTheDocument()
    expect(within(warnings).getByText("xx.json: 파일이 너무 큽니다. (최대 2 MB)")).toBeInTheDocument()
    expect(screen.queryByRole("option", { name: /ja/ })).not.toBeInTheDocument()
  })

  it("T-ui-locale-22 [Happy] 언어를 고르면 적용한 뒤 { language } 로 저장한다", async () => {
    // Arrange
    const { user, select, update } = await setup()
    // Act
    await user.click(select)
    await user.click(await screen.findByRole("option", { name: "fr" }))
    // Assert
    await waitFor(() => expect(update).toHaveBeenCalledWith({ language: "fr" }))
    expect(i18n.language).toBe("fr")
    expect(i18n.t("common.cancel")).toBe("Annuler")
  })

  it("T-ui-locale-23 [External] 적용에 실패하면 오류를 보이고 저장하지 않는다", async () => {
    // Arrange: it 는 readLocale 이 실패한다
    const { user, select, update } = await setup()
    // Act
    await user.click(select)
    await user.click(await screen.findByRole("option", { name: "it" }))
    // Assert
    expect(await screen.findByText("언어 파일을 적용하지 못해 영어로 표시합니다.")).toBeInTheDocument()
    expect(update).not.toHaveBeenCalled()
    expect(i18n.language).toBe("en")
  })

  it("T-ui-locale-24 [External] 저장에 실패하면 오류를 보이고 이전 언어로 되돌린다", async () => {
    // Arrange
    const { api, user, select, update } = await setup()
    update.mockRejectedValue({ code: "invalid_input", message: "language must be a BCP 47 code" })
    // Act
    await user.click(select)
    await user.click(await screen.findByRole("option", { name: "fr" }))
    // Assert
    expect(await screen.findByText("입력값이 올바르지 않습니다.")).toBeInTheDocument()
    await waitFor(() => expect(i18n.language).toBe("en"))
    expect(api.readLocale).toHaveBeenCalledWith("en")
  })

  it("T-ui-locale-25 [Happy] 언어 폴더 열기 버튼이 open_locale_folder 를 호출한다", async () => {
    // Arrange
    const { user, openFolder } = await setup()
    // Act
    await user.click(screen.getByRole("button", { name: "언어 폴더 열기" }))
    // Assert
    expect(openFolder).toHaveBeenCalledTimes(1)
  })

  it("T-ui-locale-26 [Happy] 목록 새로고침이 목록을 다시 조회한다", async () => {
    // Arrange
    const { api, user } = await setup()
    expect(api.listLocales).toHaveBeenCalledTimes(1)
    // Act
    await user.click(screen.getByRole("button", { name: "목록 새로고침" }))
    // Assert
    await waitFor(() => expect(api.listLocales).toHaveBeenCalledTimes(2))
  })

  it("T-ui-locale-27 [Business] 저장된 언어가 목록에 없으면 English(기본)로 표시한다", async () => {
    // Arrange + Act
    const { select } = await setup({ language: "de" })
    // Assert
    expect(select).toHaveTextContent("English (default)")
  })

  it("T-ui-locale-28 [Business] 저장된 언어가 목록에 있으면 그 언어로 표시한다", async () => {
    // Arrange + Act
    const { select } = await setup({ language: "fr" })
    // Assert
    await waitFor(() => expect(select).toHaveTextContent("fr"))
  })

  it("T-ui-locale-34 [Business] 저장된 언어가 ko 면 폴더에 ko 파일이 없어도 한국어가 선택돼 있다", async () => {
    // Arrange + Act
    const { select } = await setup({ language: "ko" })
    // Assert
    expect(select).toHaveTextContent("한국어")
  })

  it("T-ui-locale-35 [Happy] 내장 한국어를 고르면 적용하고 { language: ko } 로 저장한다", async () => {
    // Arrange
    const { user, select, update } = await setup()
    // Act
    await user.click(select)
    await user.click(await screen.findByRole("option", { name: "한국어" }))
    // Assert
    await waitFor(() => expect(update).toHaveBeenCalledWith({ language: "ko" }))
    expect(i18n.language).toBe("ko")
  })

  it("T-ui-locale-29[Validation] 오류 사유 번역 키가 모든 오류 코드에 있다", () => {
    for (const code of ["invalid_json", "invalid_shape", "too_large", "read_failed"]) {
      expect(i18n.exists(`settings.general.language.reasons.${code}`)).toBe(true)
    }
    expect(applyLanguage).toBeTypeOf("function")
  })
})
