import type { ThemeSetting } from "@/lib/ipc-contract"

export type ResolvedTheme = "light" | "gray" | "dim" | "dark"

/** 다크 계열(`.dark` 클래스·OS 다크 제목 표시줄·다크 토스트) 여부 — 라이트·그레이는 밝은 계열이다 */
export function isDarkTheme(resolved: ResolvedTheme): boolean {
  return resolved === "dim" || resolved === "dark"
}

/** localStorage 보조 캐시 키 — `public/theme-boot.js` 와 같은 값을 쓴다 */
export const THEME_CACHE_KEY = "mam.theme"

export const DEFAULT_THEME: ThemeSetting = { mode: "system", systemDark: "dim" }

/** 시스템 + OS 다크 → `systemDark`, 시스템 + OS 라이트 → 라이트(`systemDark` 무시), 그 외 → 선택값. (`public/theme-boot.js` 와 동일 로직) */
export function resolveTheme(setting: ThemeSetting, osDark: boolean): ResolvedTheme {
  if (setting.mode === "system") return osDark ? setting.systemDark : "light"
  return setting.mode
}

/** 캐시 값 검증 — 잘못되면 null (부트 스크립트와 같은 규칙) */
export function parseThemeCache(raw: string | null): ThemeSetting | null {
  if (!raw) return null
  try {
    const v = JSON.parse(raw) as Partial<ThemeSetting> | null
    const modeOk = v?.mode === "system" || v?.mode === "light" || v?.mode === "gray" || v?.mode === "dim" || v?.mode === "dark"
    const sdOk = v?.systemDark === "dim" || v?.systemDark === "dark"
    return modeOk && sdOk ? { mode: v.mode!, systemDark: v.systemDark! } : null
  } catch {
    return null
  }
}

export function readThemeCache(): ThemeSetting | null {
  try {
    return parseThemeCache(window.localStorage.getItem(THEME_CACHE_KEY))
  } catch {
    return null
  }
}
