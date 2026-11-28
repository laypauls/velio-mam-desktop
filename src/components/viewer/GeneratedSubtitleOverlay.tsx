import type { SubtitleSegment } from "@/lib/ipc-contract-ai"
import { cn } from "@/lib/utils"
import { useMpvSegment, useVideoSegment } from "./useGeneratedClock"
import type { GeneratedSubtitleUi } from "./useGeneratedSubtitle"

const NONE: readonly SubtitleSegment[] = []

/** `<video>` 경로 — rAF 로 `currentTime` 을 읽는다. 표시 중이 아니면 구간을 비워 루프·구독을 두지 않는다 */
export function HtmlGeneratedSubtitle({ el, ui, fileShown }: { el: HTMLMediaElement | null; ui: GeneratedSubtitleUi; fileShown: boolean }) {
  const segment = useVideoSegment(el, ui.shown ? ui.segments : NONE)
  return <GeneratedSubtitleOverlay segment={segment} fileShown={fileShown} />
}

/** libmpv 경로 — 위치 이벤트(`timeMs`, 최대 5Hz)를 재생 중 보간한다 */
export function MpvGeneratedSubtitle({ ui, timeMs, playing, rate, fileShown }: { ui: GeneratedSubtitleUi; timeMs: number; playing: boolean; rate: number; fileShown: boolean }) {
  const segment = useMpvSegment(ui.shown ? ui.segments : NONE, timeMs, playing, rate)
  return <GeneratedSubtitleOverlay segment={segment} fileShown={fileShown} />
}

/**
 * 플레이어 영상 위에 겹치는 생성된 자막 한 줄(부모는 `relative`). 텍스트는 React 텍스트 노드라 이스케이프가 필요 없다.
 * 위치: 자막 파일이 함께 표시되면 위쪽(파일 자막은 아래), 생성된 자막만 표시되면 아래쪽.
 * 시각 장식 겹침이라 포인터 이벤트를 받지 않는다(클릭은 재생 토글로 통과). `segment` 가 null 이면 아무것도 그리지 않는다.
 */
export function GeneratedSubtitleOverlay({ segment, fileShown }: { segment: SubtitleSegment | null; fileShown: boolean }) {
  if (segment === null) return null
  return (
    <div data-slot="generated-subtitle" data-position={fileShown ? "top" : "bottom"} className={cn("pointer-events-none absolute inset-x-0 flex justify-center px-6", fileShown ? "top-6" : "bottom-8")}>
      <p className="max-w-4/5 rounded-md bg-background/70 px-3 py-1 text-center text-base font-medium break-keep whitespace-pre-line text-foreground">{segment.text}</p>
    </div>
  )
}
