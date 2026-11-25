import { Play } from "lucide-react"
import type { ViewerPlacement } from "@/lib/ipc-contract"

function Bar({ kind }: { kind: "sidebar" | "detail" }) {
  return <div data-part={kind} className="w-4 shrink-0 rounded-sm bg-muted" />
}

function Gallery() {
  return (
    <div data-part="gallery" className="grid flex-1 grid-cols-3 content-start gap-1">
      {Array.from({ length: 6 }, (_, i) => (
        <div key={i} className="h-3 rounded-sm bg-muted" />
      ))}
    </div>
  )
}

function Player() {
  return (
    <div data-part="player" className="flex w-9 shrink-0 items-center justify-center rounded-sm border border-primary bg-primary/20 text-primary">
      <Play className="size-3" aria-hidden="true" />
    </div>
  )
}

/**
 * 플레이어 패널 위치 미리보기 — 사이드바·갤러리·플레이어·상세 네 칸의 축소 배치.
 * right: 사이드바 | 갤러리 | 플레이어 | 상세, left: 플레이어 | 사이드바 | 갤러리 | 상세. 장식이므로 호출하는 쪽(CardRadioGroup)이 aria-hidden 으로 감싼다.
 */
export function PlacementPreview({ placement }: { placement: ViewerPlacement }) {
  return (
    <div data-placement={placement} className="flex flex-1 gap-1 p-1.5">
      {placement === "left" && <Player />}
      <Bar kind="sidebar" />
      <Gallery />
      {placement === "right" && <Player />}
      <Bar kind="detail" />
    </div>
  )
}
