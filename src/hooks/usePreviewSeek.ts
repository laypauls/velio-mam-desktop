import { useEffect, useState } from "react"
import { useViewStore } from "@/stores"

/** 큰 보기 플레이어(video/audio)에 붙이는 ref — 스토어의 `seekMs` 요청이 오면 해당 시각으로 이동한다. */
export function usePreviewSeek(): (el: HTMLMediaElement | null) => void {
  const seekMs = useViewStore((s) => s.seekMs)
  const seekTick = useViewStore((s) => s.seekTick)
  const [el, setEl] = useState<HTMLMediaElement | null>(null)

  useEffect(() => {
    if (!el || seekMs === null) return
    const seconds = seekMs / 1000
    el.currentTime = seconds
    if (el.readyState >= 1) return
    // 메타데이터 로딩 전이면 로딩이 끝난 뒤 한 번 더 이동한다
    const again = () => {
      el.currentTime = seconds
    }
    el.addEventListener("loadedmetadata", again, { once: true })
    return () => el.removeEventListener("loadedmetadata", again)
  }, [el, seekMs, seekTick])

  return setEl
}
