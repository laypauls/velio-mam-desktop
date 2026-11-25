import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { setWindowFullscreen } from "@/lib/windowFullscreen"
import { useViewStore } from "@/stores"

/**
 * 스토어의 `fullscreen` 을 메인 창 전체화면에 반영한다 — 보기 영역(`ViewerPane`)이 마운트된 동안만 동작한다.
 * 창 호출이 실패하면 스토어를 창의 실제 상태로 되돌리고 toast 로 알린다(화면과 창 상태가 어긋나지 않게).
 * 보기 영역이 닫히면(어떤 경로든 언마운트) 전체화면을 해제한다.
 */
export function useWindowFullscreen() {
  const { t } = useTranslation()
  const fullscreen = useViewStore((s) => s.fullscreen)
  /** 창에 마지막으로 요청한 상태 */
  const applied = useRef(false)
  useEffect(() => {
    if (fullscreen === applied.current) return
    applied.current = fullscreen
    setWindowFullscreen(fullscreen).catch(() => {
      toast.error(t("viewer.fullscreen.failed"))
      applied.current = !fullscreen
      // 보기 영역이 이미 닫혔으면 되돌릴 화면이 없다
      if (useViewStore.getState().previewId !== null) useViewStore.setState({ fullscreen: !fullscreen })
    })
  }, [fullscreen, t])
  // 창이 숨겨지거나 최소화되면(트레이 숨김·닫기→트레이 포함) 전체화면을 풀어 다시 나타날 때 일반 창으로 보이게 한다
  useEffect(() => {
    const onVisibility = () => {
      if (document.visibilityState === "hidden") useViewStore.getState().setFullscreen(false)
    }
    document.addEventListener("visibilitychange", onVisibility)
    return () => document.removeEventListener("visibilitychange", onVisibility)
  }, [])
  useEffect(
    () => () => {
      useViewStore.setState({ fullscreen: false })
      if (applied.current) {
        applied.current = false
        void setWindowFullscreen(false).catch(() => undefined)
      }
    },
    []
  )
}
