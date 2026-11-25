import i18n, { DEFAULT_LANGUAGE, isBuiltinLanguage, rememberLanguage } from "./index"
import type { LocaleResource } from "@/lib/ipc-contract"

/** 언어 파일 내용을 읽는 함수 — `api.readLocale` */
export type ReadLocale = (code: string) => Promise<LocaleResource>

/** 표시 메타(`_meta`)는 번역 리소스가 아니므로 i18n 에 넣지 않는다 */
function stripMeta(data: LocaleResource): LocaleResource {
  return Object.fromEntries(Object.entries(data).filter(([k]) => k !== "_meta"))
}

function isPlainObject(v: unknown): v is LocaleResource {
  return typeof v === "object" && v !== null && !Array.isArray(v)
}

/** 내장 영어로 되돌린다. 실패해도 던지지 않는다. */
async function backToDefault(): Promise<void> {
  try {
    if (i18n.language !== DEFAULT_LANGUAGE) await i18n.changeLanguage(DEFAULT_LANGUAGE)
  } catch {
    // 내장 언어는 항상 번들에 있다
  }
  rememberLanguage(DEFAULT_LANGUAGE)
}

/**
 * 표시 언어를 적용한다. 던지지 않고 성공 여부를 돌려준다.
 * - `en`·`ko`(내장): 폴더에 `<code>.json` 이 있으면 내장 위에 키 단위로 덮어쓴다(없거나 읽기 실패면 내장 그대로 — 성공으로 본다).
 *   적용한 코드는 localStorage 에 기억해 다음 시작의 초기 언어로 쓴다.
 * - 그 외: 언어 파일을 읽어 리소스로 추가하고 전환한다(기억은 지운다 — 다음 시작은 `en` 으로 열리고 설정 로드 후 적용).
 *   읽기·전환에 실패하면 `en` 으로 되돌리고 false.
 */
export async function applyLanguage(code: string, read: ReadLocale): Promise<boolean> {
  if (isBuiltinLanguage(code)) {
    try {
      const data = await read(code)
      if (isPlainObject(data)) i18n.addResourceBundle(code, "translation", stripMeta(data), true, true)
    } catch {
      // 덮어쓰기 파일이 없으면 내장 언어 그대로
    }
    try {
      // 이미 그 언어면 전환하지 않는다 — languageChanged 로 앱 전체가 다시 렌더되는 것을 피한다
      if (i18n.language !== code) await i18n.changeLanguage(code)
      rememberLanguage(code)
    } catch {
      await backToDefault()
    }
    return true
  }
  try {
    const data = await read(code)
    if (!isPlainObject(data)) throw new Error("locale must be an object")
    i18n.addResourceBundle(code, "translation", stripMeta(data), true, true)
    await i18n.changeLanguage(code)
    rememberLanguage(null)
    return true
  } catch {
    await backToDefault()
    return false
  }
}

/** 숫자·날짜 서식에 쓸 현재 언어 (BCP 47). 내장 언어는 지역을 붙인다(`ko-KR`, `en-US`) */
export function currentLocale(): string {
  const lng = i18n.resolvedLanguage ?? i18n.language ?? DEFAULT_LANGUAGE
  if (lng === "ko") return "ko-KR"
  if (lng === "en") return "en-US"
  return lng
}
