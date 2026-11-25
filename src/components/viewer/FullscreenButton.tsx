import { Maximize2, Minimize2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useViewStore } from "@/stores"
import { useToggleFullscreen } from "./useToggleFullscreen"

/** 보기 영역 전체화면 토글 버튼 (컨트롤 줄·이미지 도구 줄 공용) — 단축키 F */
export function FullscreenButton() {
  const { t } = useTranslation()
  const fullscreen = useViewStore((s) => s.fullscreen)
  const toggle = useToggleFullscreen()
  const label = t(fullscreen ? "viewer.fullscreen.exit" : "viewer.fullscreen.enter")
  return (
    <Button size="icon-sm" variant="ghost" aria-label={label} title={`${label} (F)`} aria-pressed={fullscreen} onClick={toggle}>
      {fullscreen ? <Minimize2 aria-hidden="true" /> : <Maximize2 aria-hidden="true" />}
    </Button>
  )
}
