import { Fragment, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuShortcut } from "@/components/ui/context-menu"
import type { SceneCut } from "@/lib/ipc-contract"
import { ClearRangesDialog } from "./ClearRangesDialog"
import { rangeKeyLabel, type RangeKeyActionType } from "./playRangeKeys"
import { SceneClearAutoDialog } from "./SceneClearAutoDialog"
import { SceneRedetectDialog } from "./SceneRedetectDialog"
import type { RangeUi } from "./useRangeController"
import type { SceneUi } from "./useSceneController"

/** 우클릭한 위치가 가리키는 대상 — 구간(박스 위)과 장면전환점(±4px)이 겹치면 둘 다 쓴다 */
export interface TrackTarget {
  rangeIndex: number | null
  cut: SceneCut | null
}

interface MenuProps {
  ranges?: RangeUi
  scene?: SceneUi
  target: TrackTarget
}

/**
 * 구간·장면 통합 우클릭 메뉴 — 위치에 따라 항목이 달라진다.
 * 구간 위: 구간 시작으로 이동 / 이 구간 삭제. 구간 UI 가 있으면 항상: 모든 구간 삭제…. 컷 위: 이 장면전환점 이동·삭제. 장면 UI 가 있으면 항상: 다시 추출… / 자동 모두 삭제….
 * 메뉴·확인 대화상자는 Radix 포털이라 영상(캔버스) 위에 그대로 열린다.
 */
export function TrackMenuContent({ ranges, scene, target }: MenuProps) {
  const { t } = useTranslation()
  const key = (action: RangeKeyActionType) => (ranges ? (rangeKeyLabel(ranges.preset, action) ?? undefined) : undefined)
  const sourceLabel = (c: SceneCut) => t(c.source === "auto" ? "scene.sourceAuto" : "scene.sourceUser")
  const index = target.rangeIndex
  const cut = target.cut
  const autoCount = scene ? scene.cuts.filter((c) => c.source === "auto").length : 0

  const groups: ReactNode[] = []
  if (ranges && index !== null) {
    groups.push(
      <Fragment key="range">
        <ContextMenuItem onSelect={() => ranges.goToStart(index)}>
          {t("ranges.menu.goToStart")}
          <ContextMenuShortcut>{key("gotoPrev")}</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuItem variant="destructive" onSelect={() => ranges.remove(index)}>
          {t("ranges.menu.delete")}
          <ContextMenuShortcut>{key("deleteSelected")}</ContextMenuShortcut>
        </ContextMenuItem>
      </Fragment>
    )
  }
  if (ranges) {
    groups.push(
      <ContextMenuItem key="range-all" variant="destructive" disabled={ranges.ranges.length === 0} onSelect={ranges.requestClearAll}>
        {t("ranges.menu.deleteAll")}
        <ContextMenuShortcut>{key("deleteAll")}</ContextMenuShortcut>
      </ContextMenuItem>
    )
  }
  if (scene && cut) {
    groups.push(
      <Fragment key="cut">
        <ContextMenuItem onSelect={() => scene.goTo(cut)}>{t("scene.menu.goTo")}</ContextMenuItem>
        <ContextMenuItem variant="destructive" onSelect={() => scene.remove(cut)}>
          {t("scene.menu.remove")}
          <ContextMenuShortcut>{sourceLabel(cut)}</ContextMenuShortcut>
        </ContextMenuItem>
      </Fragment>
    )
  }
  if (scene) {
    groups.push(
      <Fragment key="scene">
        <ContextMenuItem disabled={scene.state === "pending"} onSelect={() => scene.setRedetectOpen(true)}>
          {t("scene.menu.redetect")}
        </ContextMenuItem>
        <ContextMenuItem variant="destructive" disabled={autoCount === 0} onSelect={() => scene.setClearAutoOpen(true)}>
          {t("scene.menu.clearAuto")}
        </ContextMenuItem>
      </Fragment>
    )
  }
  return (
    <ContextMenuContent>
      {groups.map((g, i) => (
        <Fragment key={i}>
          {i > 0 && <ContextMenuSeparator />}
          {g}
        </Fragment>
      ))}
    </ContextMenuContent>
  )
}

/** 메뉴가 여는 확인 대화상자 — 트랙 밖(포털)에 둔다 */
export function TrackDialogs({ ranges, scene }: { ranges?: RangeUi; scene?: SceneUi }) {
  const autoCount = scene ? scene.cuts.filter((c) => c.source === "auto").length : 0
  return (
    <>
      {ranges && <ClearRangesDialog open={ranges.clearAllOpen} count={ranges.ranges.length} onOpenChange={ranges.setClearAllOpen} onConfirm={ranges.confirmClearAll} />}
      {scene && (
        <>
          <SceneRedetectDialog open={scene.redetectOpen} defaultSensitivity={scene.defaultSensitivity} onOpenChange={scene.setRedetectOpen} onConfirm={scene.redetect} />
          <SceneClearAutoDialog open={scene.clearAutoOpen} count={autoCount} onOpenChange={scene.setClearAutoOpen} onConfirm={scene.confirmClearAuto} />
        </>
      )}
    </>
  )
}
