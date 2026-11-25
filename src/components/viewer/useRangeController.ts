import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useEditablePlayRanges } from "@/hooks/usePlayRanges"
import { useSettings, useUpdateSettings } from "@/hooks/useSettings"
import type { AssetDetail, PlayRange, PlayRangeKeys } from "@/lib/ipc-contract"
import { usePlaybackStore } from "@/stores"
import type { ControlsBinding } from "./MediaControls"
import type { RangeKeyActionType, RangeKeyHandlers } from "./playRangeKeys"
import { mergeRanges, minRangeMs, nextRangeEnd, prevRangeStart, rangeIndexAt, roundRanges } from "./playRanges"

/** 컨트롤 바(구간 레인·토글)가 쓰는 상태와 동작 */
export interface RangeUi {
  ranges: readonly PlayRange[]
  selected: number | null
  /** 대기 In/Out (ms) — 확정 전 표시 */
  pendingInMs: number | null
  pendingOutMs: number | null
  minMs: number
  preset: PlayRangeKeys
  /** 지금 이 항목에 적용되는 "구간만 재생" 값 (연속 재생 중이면 컬렉션 설정) */
  rangesOnly: boolean
  /** 단독 재생일 때만 컨트롤 바에서 토글한다 — 연속 재생 중에는 재생 바의 토글이 맡는다 */
  canToggleRangesOnly: boolean
  saveFailed: boolean
  clearAllOpen: boolean
  setClearAllOpen: (open: boolean) => void
  confirmClearAll: () => void
  create: (range: PlayRange) => void
  adjust: (index: number, range: PlayRange) => void
  select: (index: number | null) => void
  goToStart: (index: number) => void
  remove: (index: number) => void
  requestClearAll: () => void
  toggleRangesOnly: () => void
  /** 구간 시작/끝 표시 — 프리셋의 시작/끝 키와 같은 동작(`keys.run`) */
  markIn: () => void
  markOut: () => void
}

export interface RangeController {
  ui: RangeUi
  keys: RangeKeyHandlers
  /**
   * 재생 쪽에 강제할 구간 — `null` = 전체 재생(끔·구간 없음·D7), 배열 = 구간만 재생,
   * `undefined` = 켜져 있지만 구간을 아직 못 받음(판단 보류)
   */
  activeRanges: readonly PlayRange[] | null | undefined
}

/**
 * 영상·음성 구간 편집의 중심 — 서버 구간(디바운스 저장), 선택·대기 In/Out 상태, 구간 키 동작, "구간만 재생" 설정을 묶는다.
 * 재생 엔진(`<video>` / libVLC)은 `controls` 어댑터로만 접근하므로 두 경로가 같은 코드를 쓴다.
 */
export function useRangeController(asset: AssetDetail, controls: ControlsBinding): RangeController {
  const { t } = useTranslation()
  const settings = useSettings().data
  const updateSettings = useUpdateSettings({ silent: true })
  const { ranges, loaded, commit, saveFailed } = useEditablePlayRanges(asset.assetId)
  // 연속 재생 맥락 — 재생 바에서 일시정지해도(paused) 같은 항목이면 컬렉션 설정·구간 강제를 유지한다 (자동 재생 판단은 useOpenAutoplay 가 "playing" 만 본다)
  const inCollection = usePlaybackStore((s) => (s.status === "playing" || s.status === "paused") && s.currentId === asset.assetId)
  const [selectedRaw, setSelected] = useState<number | null>(null)
  const [pendingInMs, setPendingIn] = useState<number | null>(null)
  const [pendingOutMs, setPendingOut] = useState<number | null>(null)
  const [clearAllOpen, setClearAllOpen] = useState(false)

  const { state } = controls
  const durationMs = Number.isFinite(state.duration) && state.duration > 0 ? state.duration * 1000 : (asset.durationMs ?? 0)
  const minMs = minRangeMs(asset.fps)
  const preset: PlayRangeKeys = settings?.playRangeKeys ?? "brackets"
  const selected = selectedRaw !== null && selectedRaw < ranges.length ? selectedRaw : null
  const nowMs = () => Math.round(state.currentTime * 1000)

  /** 검증을 통과한 구간 목록으로 교체하고, 새·수정 구간(병합 결과)을 선택한다 */
  const apply = (next: PlayRange[], focusMs: number | null) => {
    commit(next)
    const merged = mergeRanges(roundRanges(next))
    setSelected(focusMs === null ? null : rangeIndexAt(merged, focusMs))
  }

  /** 구간 후보 검증 — 실패하면 안내만 하고 아무것도 바꾸지 않는다 */
  const validate = (r: PlayRange): PlayRange | null => {
    const upper = durationMs > 0 ? durationMs : Number.POSITIVE_INFINITY
    const startMs = Math.round(Math.max(0, r.startMs))
    const endMs = Math.round(Math.min(upper, r.endMs))
    if (endMs <= startMs) {
      toast.warning(t("ranges.outBeforeIn"))
      return null
    }
    if (endMs - startMs < minMs) {
      toast.warning(t("ranges.tooShort", { ms: minMs }))
      return null
    }
    return { startMs, endMs }
  }

  const create = (r: PlayRange) => {
    const ok = validate(r)
    if (ok) apply([...ranges, ok], (ok.startMs + ok.endMs) / 2)
  }
  const adjust = (index: number, r: PlayRange) => {
    const ok = validate(r)
    if (ok) apply(ranges.map((x, i) => (i === index ? ok : x)), (ok.startMs + ok.endMs) / 2)
  }
  const remove = (index: number) => apply(ranges.filter((_, i) => i !== index), null)
  const confirmClearAll = () => {
    setPendingIn(null)
    setPendingOut(null)
    apply([], null)
  }
  const requestClearAll = () => {
    if (ranges.length > 0) setClearAllOpen(true)
  }
  const goToStart = (index: number) => controls.onSeek(ranges[index].startMs / 1000)

  /** In/Out 짝이 맞으면 구간으로 확정하고 대기 표시를 지운다 (검증에 실패하면 대기 표시를 유지) */
  const confirmPending = (startMs: number, endMs: number) => {
    const ok = validate({ startMs, endMs })
    if (!ok) return
    setPendingIn(null)
    setPendingOut(null)
    apply([...ranges, ok], (ok.startMs + ok.endMs) / 2)
  }

  const run = (action: RangeKeyActionType) => {
    const now = nowMs()
    switch (action) {
      case "markIn":
        return pendingOutMs !== null ? confirmPending(now, pendingOutMs) : setPendingIn(now)
      case "markOut":
        return pendingInMs !== null ? confirmPending(pendingInMs, now) : setPendingOut(now)
      case "gotoPrev": {
        const to = prevRangeStart(ranges, now)
        return to === null ? undefined : controls.onSeek(to / 1000)
      }
      case "gotoNext": {
        const to = nextRangeEnd(ranges, now)
        return to === null ? undefined : controls.onSeek(to / 1000)
      }
      case "selectAtPlayhead": {
        const at = rangeIndexAt(ranges, now)
        return at < 0 ? undefined : setSelected(at)
      }
      case "clearIn":
        return setPendingIn(null)
      case "clearOut":
        return setPendingOut(null)
      case "deleteAll":
        return requestClearAll()
      case "deleteSelected":
        return selected === null ? undefined : remove(selected)
    }
  }

  const wanted = inCollection ? settings?.collectionPlayRangesOnly === true : settings?.playRangesOnly === true
  const activeRanges = !wanted ? null : !loaded ? undefined : ranges.length > 0 ? ranges : null

  return {
    activeRanges,
    keys: { preset, hasSelection: selected !== null, run },
    ui: {
      ranges,
      selected,
      pendingInMs,
      pendingOutMs,
      minMs,
      preset,
      rangesOnly: wanted,
      canToggleRangesOnly: !inCollection,
      saveFailed,
      clearAllOpen,
      setClearAllOpen,
      confirmClearAll,
      create,
      adjust,
      select: setSelected,
      goToStart,
      remove,
      requestClearAll,
      toggleRangesOnly: () => updateSettings.mutate({ playRangesOnly: !wanted }),
      markIn: () => run("markIn"),
      markOut: () => run("markOut"),
    },
  }
}
