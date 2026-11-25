import { useEffect, useState } from "react"
import { useApi } from "@/lib/api"

/**
 * 영상을 지금 그려야 하는가 — 보기 영역이 보이고(`stageVisible`) 창이 숨겨지지 않았을 때(`document.hidden` 아님).
 * `ready`(세션이 열림)가 되면 현재 값을 호스트에 한 번 알리고, 이후 바뀔 때마다 `playerSetRenderActive` 를 보낸다
 * (워커는 비활성 동안 프레임을 만들지 않는다 — 최소화는 Rust 쪽 창 이벤트도 같은 일을 한다).
 * 스냅샷·창 숨김은 없다: 캔버스는 일반 DOM 요소라 메뉴·대화상자가 위에 그대로 열린다.
 */
export function useRenderActive(ready: boolean, stageVisible: boolean): boolean {
  const api = useApi()
  const [pageHidden, setPageHidden] = useState(() => document.hidden)
  useEffect(() => {
    const sync = () => setPageHidden(document.hidden)
    document.addEventListener("visibilitychange", sync)
    sync()
    return () => document.removeEventListener("visibilitychange", sync)
  }, [])
  const active = stageVisible && !pageHidden
  useEffect(() => {
    if (ready) api.playerSetRenderActive(active).catch(() => undefined)
  }, [api, ready, active])
  return active
}
