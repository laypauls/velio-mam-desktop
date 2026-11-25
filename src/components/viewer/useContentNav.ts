import { usePlaybackStore, useViewerNavStore } from "@/stores"
import type { ContentNav } from "./PlayControls"

/** 컨트롤 바의 이전/다음 콘텐츠 + 그 단축키(Shift+P/N)를 단독 보기에서 처리해야 하는지 */
export interface ContentNavSource extends ContentNav {
  /** true = 단독 보기(갤러리 순서). false = 연속 재생 — 단축키는 `usePlaybackKeys` 가 처리한다 */
  standalone: boolean
}

/**
 * 이전/다음 콘텐츠 이동의 공급원 — 연속 재생 중이면 재생 대기열(`PlaybackBar` 의 이전/다음과 같은 동작, 처음에서 반복 끄기면 이전 비활성,
 * 끝에서 반복 끄기면 다음 비활성), 아니면 갤러리의 현재 결과 순서에서 열린 항목의 이전/다음(갤러리 화살표 키와 같은 경로).
 * 이동할 대상이 없으면(갤러리 밖에서 열었거나 결과가 1개) undefined — 버튼을 그리지 않는다.
 */
export function useContentNav(): ContentNavSource | undefined {
  const status = usePlaybackStore((s) => s.status)
  const index = usePlaybackStore((s) => s.index)
  const total = usePlaybackStore((s) => s.queue.length)
  const repeat = usePlaybackStore((s) => s.repeat)
  const stepper = useViewerNavStore((s) => s.stepper)
  if (status !== "idle") {
    const { prev, next } = usePlaybackStore.getState()
    return { canPrev: !(index <= 0 && repeat === "off"), canNext: !(index >= total - 1 && repeat === "off"), onPrev: prev, onNext: next, standalone: false }
  }
  if (stepper === null) return undefined
  return { canPrev: stepper.canPrev, canNext: stepper.canNext, onPrev: () => stepper.step(-1), onNext: () => stepper.step(1), standalone: true }
}
