import { useMemo, useRef, useState } from "react"
import { useSceneCuts, useSceneDetect, useSceneEdit } from "@/hooks/useSceneCuts"
import { useSettings, useUpdateSettings } from "@/hooks/useSettings"
import type { AssetDetail, SceneCut, SceneFailure, SceneSensitivity, SceneState } from "@/lib/ipc-contract"
import type { ControlsBinding } from "./MediaControls"
import { SCENE_NAV_BASE_WINDOW_MS, halfFrameMs, sceneNav, type SceneKeyHandlers } from "./sceneKeys"

/** "다시 추출…" Dialog 의 선택값 */
export interface RedetectChoice {
  sensitivity: SceneSensitivity
  restoreSuppressed: boolean
}

/** 컨트롤 바(눈금 레이어·토글·추출 버튼·메뉴·Dialog)가 쓰는 상태와 동작 */
export interface SceneUi {
  /** ms 오름차순, 자동·사용자 병합 */
  cuts: readonly SceneCut[]
  state: SceneState
  /** 마지막 추출의 민감도 — null 이면 추출한 적 없음(사용자 컷만 있는 행 포함) */
  sensitivity: SceneSensitivity | null
  error: SceneFailure | null
  durationMs: number
  /** 설정 `sceneCutsVisible` — 끄면 눈금과 hover 만 사라지고 추출·메뉴는 그대로 */
  visible: boolean
  defaultSensitivity: SceneSensitivity
  /** "추출" 버튼을 보일 상태 — 행 없음, 또는 사용자 컷만 있는 행(done + 민감도 null) */
  canExtract: boolean
  toggleVisible: () => void
  /** 이전/다음 장면전환점으로 이동 — `Ctrl+←/→` 와 같은 처리(`keys.move`). 표시 토글과 무관하게 동작한다 */
  move: (dir: 1 | -1) => void
  /** 현재 위치 앞/뒤에 장면전환점이 있는가 (`sceneNav` 가 null 이 아님) — 없으면 이동 버튼을 비활성화한다 */
  canMovePrev: boolean
  canMoveNext: boolean
  extract: () => void
  goTo: (cut: SceneCut) => void
  remove: (cut: SceneCut) => void
  redetectOpen: boolean
  setRedetectOpen: (open: boolean) => void
  redetect: (choice: RedetectChoice) => void
  clearAutoOpen: boolean
  setClearAutoOpen: (open: boolean) => void
  confirmClearAuto: () => void
}

export interface SceneController {
  /** 영상이 아니면 undefined — 장면 UI·키를 쓰지 않는다 */
  ui: SceneUi | undefined
  keys: SceneKeyHandlers | undefined
}

/**
 * 영상 한 편의 장면전환점 중심 — 서버 컷(추출 완료 이벤트로 갱신), 표시 설정, 추출·재추출·삭제, `Ctrl+←/→`·`C` 키 동작을 묶는다.
 * 재생 엔진(`<video>` / libmpv)은 `controls` 어댑터로만 접근하므로 두 경로가 같은 코드를 쓴다. `readTimeMs` 는 요소가 더 정확한 현재 시각을 알 때만 준다.
 * 이동 기준은 "마지막 이동 목표(1초 안)", 없으면 현재 시각 — 플레이어가 보고하는 시각은 최대 5Hz 로 갱신돼 연타하면 같은 컷으로 되돌아가기 때문이다.
 * 같은 기준을 `C` 추가에도 써서, 이동 직후 추가해도 방금 본 위치에 컷이 생긴다.
 */
export function useSceneController(asset: AssetDetail, controls: ControlsBinding, readTimeMs?: () => number): SceneController {
  const isVideo = asset.mediaType === "video"
  const settings = useSettings().data
  const updateSettings = useUpdateSettings({ silent: true })
  const query = useSceneCuts(asset.assetId, isVideo)
  const edit = useSceneEdit(asset.assetId)
  const detect = useSceneDetect()
  const [redetectOpen, setRedetectOpen] = useState(false)
  const [clearAutoOpen, setClearAutoOpen] = useState(false)
  const lastTarget = useRef<{ ms: number; at: number } | null>(null)

  const data = query.data
  const cuts = useMemo<readonly SceneCut[]>(() => data?.cuts ?? [], [data])
  const cutMs = useMemo(() => cuts.map((c) => c.ms), [cuts])
  const { state } = controls
  const durationMs = Number.isFinite(state.duration) && state.duration > 0 ? state.duration * 1000 : (asset.durationMs ?? 0)
  const sceneState: SceneState = data?.state ?? "none"
  const sensitivity = data?.sensitivity ?? null
  const hf = halfFrameMs(asset.fps)
  const shownMs = Math.round(state.currentTime * 1000)

  const baseMs = (): number => {
    const last = lastTarget.current
    if (last !== null && Date.now() - last.at <= SCENE_NAV_BASE_WINDOW_MS) return last.ms
    return Math.round(readTimeMs ? readTimeMs() : state.currentTime * 1000)
  }
  const seekMs = (ms: number) => {
    lastTarget.current = { ms, at: Date.now() }
    controls.onSeek((durationMs > 0 ? Math.min(ms, durationMs) : ms) / 1000)
  }

  const keys: SceneKeyHandlers = {
    move: (dir) => {
      const to = sceneNav(cutMs, baseMs(), dir, asset.fps)
      if (to !== null) seekMs(to)
    },
    add: () => {
      const at = baseMs()
      edit.mutate({ type: "add", ms: durationMs > 0 ? Math.min(at, durationMs) : at })
    },
  }

  const ui: SceneUi = {
    cuts,
    state: sceneState,
    sensitivity,
    error: data?.error ?? null,
    durationMs,
    visible: settings?.sceneCutsVisible ?? true,
    defaultSensitivity: settings?.sceneDetectSensitivity ?? "medium",
    canExtract: query.isSuccess && (sceneState === "none" || (sceneState === "done" && sensitivity === null)),
    toggleVisible: () => updateSettings.mutate({ sceneCutsVisible: !(settings?.sceneCutsVisible ?? true) }),
    move: keys.move,
    canMovePrev: cutMs.length > 0 && sceneNav(cutMs, shownMs, -1, asset.fps) !== null,
    canMoveNext: cutMs.length > 0 && sceneNav(cutMs, shownMs, 1, asset.fps) !== null,
    // 사용자 컷만 있는 행은 서버가 "이미 done" 으로 건너뛰므로 redetect 로 보낸다
    extract: () => detect.mutate({ assetIds: [asset.assetId], options: sceneState === "done" ? { redetect: true } : {} }),
    goTo: (cut) => seekMs(cut.ms + hf),
    remove: (cut) => edit.mutate({ type: "remove", ms: cut.ms, source: cut.source }),
    redetectOpen,
    setRedetectOpen,
    redetect: (choice) =>
      detect.mutate({ assetIds: [asset.assetId], options: { redetect: true, restoreSuppressed: choice.restoreSuppressed, sensitivity: choice.sensitivity } }),
    clearAutoOpen,
    setClearAutoOpen,
    confirmClearAuto: () => edit.mutate({ type: "clearAuto" }),
  }
  return isVideo ? { ui, keys } : { ui: undefined, keys: undefined }
}
