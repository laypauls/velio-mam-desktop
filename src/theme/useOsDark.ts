import { useEffect, useState } from "react"

const QUERY = "(prefers-color-scheme: dark)"

function query(): MediaQueryList | null {
  return typeof window !== "undefined" && typeof window.matchMedia === "function" ? window.matchMedia(QUERY) : null
}

/** Windows 앱 모드가 다크인지 — `change` 를 구독해 실행 중 전환을 반영한다. */
export function useOsDark(): boolean {
  const [dark, setDark] = useState(() => query()?.matches ?? false)
  useEffect(() => {
    const mq = query()
    if (!mq) return
    const onChange = (e: MediaQueryListEvent) => setDark(e.matches)
    setDark(mq.matches)
    mq.addEventListener("change", onChange)
    return () => mq.removeEventListener("change", onChange)
  }, [])
  return dark
}
