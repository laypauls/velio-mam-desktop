import { useEffect, useState } from "react"
import { OPEN_LAYER } from "@/lib/openLayer"

/** 전체화면에서 마우스가 멈춘 뒤 컨트롤·커서를 숨기기까지(ms) */
export const FULLSCREEN_IDLE_MS = 2500
const CONTROLS_HOVER = '[data-player-controls]:hover, [role="toolbar"]:hover'

/**
 * 전체화면(`active`)에서 마우스가 `FULLSCREEN_IDLE_MS` 동안 움직이지 않으면 true.
 * 포인터가 컨트롤 위에 있거나 메뉴·대화상자가 열려 있으면 숨기지 않는다. 키 입력은 재생 키 동작에 영향이 없으므로 보이기만 하지 않는다.
 * `active` 가 꺼지면 즉시 false.
 */
export function useFullscreenIdle(active: boolean): boolean {
  const [idle, setIdle] = useState(false)
  useEffect(() => {
    if (!active) {
      setIdle(false)
      return
    }
    let timer: ReturnType<typeof setTimeout>
    const arm = () => {
      clearTimeout(timer)
      timer = setTimeout(() => {
        const busy = document.querySelector(CONTROLS_HOVER) !== null || document.querySelector(OPEN_LAYER) !== null
        if (busy) arm()
        else setIdle(true)
      }, FULLSCREEN_IDLE_MS)
    }
    const wake = () => {
      setIdle(false)
      arm()
    }
    wake()
    window.addEventListener("mousemove", wake)
    window.addEventListener("pointerdown", wake)
    window.addEventListener("wheel", wake)
    return () => {
      clearTimeout(timer)
      window.removeEventListener("mousemove", wake)
      window.removeEventListener("pointerdown", wake)
      window.removeEventListener("wheel", wake)
    }
  }, [active])
  return idle
}
