import { useEffect, useMemo } from "react"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { useSettings } from "@/hooks/useSettings"
import { applyTheme, cacheTheme } from "./apply"
import { DEFAULT_THEME, isDarkTheme, readThemeCache, resolveTheme, type ResolvedTheme } from "./resolve"
import { useResolvedThemeStore } from "./store"
import { useOsDark } from "./useOsDark"

/** Windows 제목 표시줄을 앱 테마에 맞춘다(딤은 OS 쪽 dark). Tauri 가 아니거나 실패하면 조용히 무시한다. */
export function syncWindowTheme(resolved: ResolvedTheme): void {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return
  try {
    getCurrentWindow()
      .setTheme(isDarkTheme(resolved) ? "dark" : "light")
      .catch(() => undefined)
  } catch {
    // 제목 표시줄 동기화는 부가 기능
  }
}

/** 앱 셸에서 한 번 호출 — 설정(없으면 localStorage 캐시, 그것도 없으면 기본값)을 해석해 `<html>` 에 반영한다. */
export function useThemeSync(): void {
  const { data } = useSettings()
  const osDark = useOsDark()
  const cached = useMemo(() => readThemeCache(), [])
  const mode = data?.theme.mode ?? cached?.mode ?? DEFAULT_THEME.mode
  const systemDark = data?.theme.systemDark ?? cached?.systemDark ?? DEFAULT_THEME.systemDark
  const resolved = resolveTheme({ mode, systemDark }, osDark)
  const setResolved = useResolvedThemeStore((s) => s.set)

  useEffect(() => {
    applyTheme(resolved)
    setResolved(resolved)
    syncWindowTheme(resolved)
  }, [resolved, setResolved])

  // SQLite 값이 로드되면 보조 캐시를 갱신한다
  const loaded = data?.theme
  useEffect(() => {
    if (loaded) cacheTheme(loaded)
  }, [loaded])
}
