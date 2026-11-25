import type { ViewerPlacement } from "@/lib/ipc-contract"
import { useSettings } from "./useSettings"

/** 플레이어 패널 위치 (설정 `viewerPlacement`). 설정을 아직 못 받았거나 알 수 없는 값이면 기본 "left" */
export function useViewerPlacement(): ViewerPlacement {
  return useSettings().data?.viewerPlacement === "right" ? "right" : "left"
}
