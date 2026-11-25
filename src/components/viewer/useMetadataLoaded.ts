import { useEffect, useState } from "react"

/**
 * 미디어 요소가 메타데이터를 읽었고(`loadedmetadata` / `readyState >= HAVE_METADATA`) 오류가 없는 동안 true.
 * 원본이 실제로 열렸다는 신호 — 자막 탐색처럼 원본 폴더를 읽는 부수 작업의 시작 조건. `src` 가 바뀌면 다시 기다린다.
 */
export function useMetadataLoaded(el: HTMLMediaElement | null, src: string): boolean {
  const [state, setState] = useState({ loaded: false, failed: false })
  useEffect(() => {
    setState({ loaded: false, failed: false })
    if (!el) return
    const onLoaded = () => setState((s) => ({ ...s, loaded: true }))
    const onError = () => setState((s) => ({ ...s, failed: true }))
    if (el.readyState >= 1) onLoaded()
    el.addEventListener("loadedmetadata", onLoaded)
    el.addEventListener("error", onError)
    return () => {
      el.removeEventListener("loadedmetadata", onLoaded)
      el.removeEventListener("error", onError)
    }
  }, [el, src])
  return state.loaded && !state.failed
}
