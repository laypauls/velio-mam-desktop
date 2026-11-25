import { useSettings } from "./useSettings"
import { displayName, nameTooltip, type Titled } from "@/lib/displayName"

/** 자산 표시 이름 — 설정 `titleEditEnabled` 가 켜져 있고 제목이 있으면 제목, 아니면 파일명. 실제 파일명이 필요한 곳은 `fileName` 을 그대로 쓴다. */
export function useDisplayName() {
  const enabled = useSettings().data?.titleEditEnabled ?? false
  return {
    enabled,
    nameOf: (a: Titled) => displayName(a, enabled),
    tooltipOf: (a: Titled) => nameTooltip(a, enabled),
  }
}
