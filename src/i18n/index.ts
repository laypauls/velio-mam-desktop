import i18n from "i18next"
import { initReactI18next } from "react-i18next"
import en from "../../src-tauri/locales/en.json"
import ko from "./locales/ko.json"

/** 기본·폴백 언어 — 설정이 없거나 언어 파일 적용에 실패하면 이 언어로 돌아간다 */
export const DEFAULT_LANGUAGE = "en"
/** 번들에 항상 들어 있는 언어. 언어 폴더의 같은 코드 파일은 이 위에 덮어쓰기로 쓴다 */
export const BUILTIN_LANGUAGES = ["en", "ko"] as const
export type BuiltinLanguage = (typeof BUILTIN_LANGUAGES)[number]

export function isBuiltinLanguage(code: string): code is BuiltinLanguage {
  return (BUILTIN_LANGUAGES as readonly string[]).includes(code)
}

/** 마지막으로 적용한 내장 언어 — 설정을 IPC 로 읽기 전의 첫 화면 깜빡임을 줄이는 per-viewer 편의 값 */
const STORAGE_KEY = "mam.uiLanguage"

function readStoredLanguage(): BuiltinLanguage {
  try {
    const v = window.localStorage.getItem(STORAGE_KEY)
    if (v !== null && isBuiltinLanguage(v)) return v
  } catch {
    // localStorage 접근 불가(비활성·샌드박스) — 기본 언어로 시작
  }
  return DEFAULT_LANGUAGE
}

/** 적용한 언어를 기억한다. 내장 언어면 코드를, 폴더 언어팩(`null`)이면 기억을 지운다. 실패는 무시한다. */
export function rememberLanguage(code: BuiltinLanguage | null): void {
  try {
    if (code === null) window.localStorage.removeItem(STORAGE_KEY)
    else window.localStorage.setItem(STORAGE_KEY, code)
  } catch {
    // 기억하지 못해도 동작에는 영향 없음
  }
}

/** 표시 메타(`_meta`)는 번역 리소스가 아니므로 i18n 에 넣지 않는다 */
const { _meta: _enMeta, ...enResource } = en
void _enMeta

void i18n.use(initReactI18next).init({
  resources: { en: { translation: enResource }, ko: { translation: ko } },
  lng: readStoredLanguage(),
  fallbackLng: DEFAULT_LANGUAGE,
  interpolation: { escapeValue: false },
})

// `<html lang>` 를 현재 언어와 맞춘다 (스크린리더·하이픈·폰트 선택)
if (typeof document !== "undefined") {
  document.documentElement.lang = i18n.language || DEFAULT_LANGUAGE
  i18n.on("languageChanged", (lng) => {
    document.documentElement.lang = lng
  })
}

export default i18n
