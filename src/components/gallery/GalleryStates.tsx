import { FolderPlus, SearchX } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useAddFolder } from "@/hooks/useAddFolder"
import { useSearchStore, useViewStore } from "@/stores"
import { CARD_SIZES, gridLayout } from "./cardDisplay"
import { AddFolderDialog } from "@/components/folders/AddFolderDialog"
import { EmptyState } from "./EmptyState"
import { GAP, PAD } from "./galleryGeometry"

/** 첫 결과를 기다리는 자리표시. 폭을 아직 모르면(0) 기본 폭으로 */
export function LoadingSkeleton({ width }: { width: number }) {
  const { t } = useTranslation()
  const cardSize = useViewStore((s) => s.cardDisplay.cardSize)
  const { cols, cell: exact } = gridLayout(Math.max(0, (width || 1000) - PAD * 2), CARD_SIZES[cardSize], GAP)
  const cell = Math.floor(exact * 100) / 100 // 부동소수 오차로 마지막 칸이 다음 줄로 넘어가지 않게
  return (
    // 로딩 자리표시도 실제 격자와 같은 열 수·폭으로 — 로딩이 끝날 때 크기가 튀지 않게 한다
    <div role="status" aria-busy="true" aria-label={t("common.loading")} className="flex flex-wrap gap-2 overflow-hidden p-3">
      {Array.from({ length: cols * 4 }, (_, i) => (
        <Skeleton key={i} className="rounded-lg" style={{ width: cell, height: cell }} />
      ))}
    </div>
  )
}

/** 결과가 0건일 때: 필터/검색어가 있으면 조건 초기화, 없으면 폴더 추가 안내 */
export function GalleryEmpty({ filtered }: { filtered: boolean }) {
  const { t } = useTranslation()
  const resetFilters = useSearchStore((s) => s.resetFilters)
  const setQuery = useSearchStore((s) => s.setQuery)
  const addFolder = useAddFolder()
  return filtered ? (
    <EmptyState
      icon={SearchX}
      title={t("gallery.emptyFiltered.title")}
      description={t("gallery.emptyFiltered.description")}
      action={
        <Button
          variant="outline"
          onClick={() => {
            resetFilters()
            setQuery("")
          }}
        >
          {t("gallery.emptyFiltered.action")}
        </Button>
      }
    />
  ) : (
    <>
      <EmptyState
        icon={FolderPlus}
        title={t("gallery.empty.title")}
        description={t("gallery.empty.description")}
        action={
          <Button onClick={() => addFolder.start()} disabled={addFolder.isPending}>
            {t("gallery.empty.action")}
          </Button>
        }
      />
      <AddFolderDialog flow={addFolder} />
    </>
  )
}
