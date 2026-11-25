import { Pointer } from "lucide-react"
import type { GalleryPaging, IndexRailMode } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"

/** 미리보기 카드 격자 — 갤러리 카드를 흉내 낸 작은 상자들. 장식이므로 호출하는 쪽(CardRadioGroup)이 aria-hidden 으로 감싼다. */
function Cells({ count, className }: { count: number; className?: string }) {
  return (
    <div className={cn("grid grid-cols-4 content-start gap-1", className)}>
      {Array.from({ length: count }, (_, i) => (
        <div key={i} className="h-3 rounded-sm bg-muted" />
      ))}
    </div>
  )
}

/** 무한 스크롤: 카드가 아래로 이어지고 오른쪽에 스크롤바 / 페이지: 카드 아래에 쪽 번호 */
export function PagingPreview({ mode }: { mode: GalleryPaging }) {
  if (mode === "infinite") {
    return (
      <div className="relative flex-1 p-1.5 pr-3">
        <Cells count={12} />
        <div className="absolute inset-y-1 right-1 w-1 rounded-sm bg-muted">
          <div className="h-1/3 rounded-sm bg-muted-foreground/60" />
        </div>
      </div>
    )
  }
  return (
    <div className="flex flex-1 flex-col justify-between p-1.5">
      <Cells count={8} />
      <div className="flex items-center justify-center gap-1">
        {[0, 1, 2, 3, 4].map((i) => (
          <div key={i} className={cn("h-1.5 w-2.5 rounded-sm", i === 1 ? "bg-primary" : "bg-muted-foreground/40")} />
        ))}
      </div>
    </div>
  )
}

const RAIL_LABELS = [0, 1, 2, 3]

/**
 * 목차 레일: 세 모드 모두 오른쪽 레일에 위치 핸들이 있다.
 * 항상 = 목차 라벨이 계속 보임 / 자동 = 스크롤·마우스 오버·드래그 중에만 잠시 보임(흐린 점선 + 포인터 아이콘) / 끄기 = 라벨 없이 핸들만.
 */
export function RailPreview({ mode }: { mode: IndexRailMode }) {
  return (
    <div className="flex flex-1">
      <div className="flex-1 p-1.5">
        <Cells count={8} />
      </div>
      <div className="relative flex w-9 items-stretch justify-end gap-1 border-l border-border bg-muted/40 py-1.5 pr-1">
        {mode !== "off" && (
          <div className="flex flex-col items-end justify-between py-0.5">
            {RAIL_LABELS.map((i) => (
              <div
                key={i}
                className={cn("h-1.5 w-3.5 rounded-sm", mode === "always" ? "bg-foreground/70" : "border border-dashed border-muted-foreground/60 opacity-50")}
              />
            ))}
          </div>
        )}
        <div className="relative w-1 rounded-sm bg-muted">
          <div className="absolute top-1/4 h-3 w-full rounded-sm bg-primary" />
        </div>
        {mode === "auto" && <Pointer className="absolute bottom-0.5 left-0.5 size-3 text-muted-foreground" aria-hidden="true" />}
      </div>
    </div>
  )
}
