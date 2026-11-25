import type { ThemeSetting } from "@/lib/ipc-contract"
import { THEME_CACHE_KEY, isDarkTheme, type ResolvedTheme } from "./resolve"

/** `<html>` 에 해석된 테마를 적용한다. 딤도 `.dark` 를 붙인다(`dark:` 변형 유지). 값이 같으면 DOM 을 건드리지 않는다. */
export function applyTheme(resolved: ResolvedTheme): void {
  const root = document.documentElement
  if (root.getAttribute("data-theme") !== resolved) root.setAttribute("data-theme", resolved)
  const dark = isDarkTheme(resolved)
  if (root.classList.contains("dark") !== dark) root.classList.toggle("dark", dark)
}

/** 설정을 localStorage 보조 캐시에 쓴다(원천은 SQLite). 저장소가 막혀 있어도 던지지 않는다. */
export function cacheTheme(setting: ThemeSetting): void {
  try {
    const json = JSON.stringify({ mode: setting.mode, systemDark: setting.systemDark })
    if (window.localStorage.getItem(THEME_CACHE_KEY) !== json) window.localStorage.setItem(THEME_CACHE_KEY, json)
  } catch {
    // 보조 캐시일 뿐 — 실패해도 화면 적용에는 영향 없음
  }
}
