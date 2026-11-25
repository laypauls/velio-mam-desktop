import { toast } from "sonner"
import { create } from "zustand"
import i18n from "@/i18n"
import type { PlayQueueItem } from "@/lib/ipc-contract"
import { useViewStore } from "./view"

export type PlaybackStatus = "idle" | "playing" | "paused" | "stopped"
export type RepeatMode = "off" | "all"

/** 컬렉션 연속 재생 상태 — 대기열은 컬렉션 순서 전체(갤러리의 정렬·검색·필터와 무관). */
interface PlaybackState {
  collectionId: number | null
  queue: readonly PlayQueueItem[]
  index: number
  currentId: number | null
  /** idle: 재생 안 함 / playing / paused: 이미지 타이머 일시정지 / stopped: 반복 끄기로 끝까지 재생(보기 영역은 마지막 항목 유지) */
  status: PlaybackStatus
  repeat: RepeatMode
  /** 항목이 (다시) 선택될 때마다 증가 — 같은 항목 반복에서도 뷰어를 새로 마운트하는 키 */
  token: number
  failed: ReadonlySet<number>
  consecutiveFailures: number
  /** 대기열을 받은 뒤 시작한다 — `fromAssetId` 가 재생할 수 없으면 그 뒤에서 이어서 찾는다 */
  start: (collectionId: number, queue: readonly PlayQueueItem[], fromAssetId?: number) => void
  next: () => void
  prev: () => void
  /** 대기열 안의 항목으로 이동해 거기서부터 이어서 재생 (재생할 수 없으면 뒤로 건너뜀) */
  jumpTo: (assetId: number) => void
  /** 현재 항목이 끝났음 — 현재 항목이 아닌 보고는 무시한다 */
  reportEnded: (assetId: number) => void
  /** 현재 항목을 재생하지 못함 — 건너뛰고 다음으로 */
  reportFailed: (assetId: number) => void
  /** 순서·구성이 바뀐 대기열 — 현재 항목 위치를 다시 찾는다 */
  setQueue: (queue: readonly PlayQueueItem[]) => void
  toggleRepeat: () => void
  /** 이미지 타이머 일시정지/재개 */
  togglePause: () => void
  /** 갤러리 카드 클릭 — Ctrl/Shift 는 선택만, 평범한 클릭은 대기열 항목이면 그 항목으로 이동 */
  clickAsset: (assetId: number, mods: { ctrl: boolean; shift: boolean }) => void
  stop: () => void
}

const EMPTY: ReadonlySet<number> = new Set()

const IDLE = {
  collectionId: null,
  queue: [] as readonly PlayQueueItem[],
  index: -1,
  currentId: null,
  status: "idle" as PlaybackStatus,
  failed: EMPTY,
  consecutiveFailures: 0,
}

type Found = { kind: "found"; index: number } | { kind: "end" } | { kind: "none" }

/** `from` 다음부터 `dir` 방향으로 재생할 수 있는 첫 항목. 반복 끄기에서 끝을 넘으면 end, 한 바퀴 돌아도 없으면 none */
function find(queue: readonly PlayQueueItem[], failed: ReadonlySet<number>, from: number, dir: 1 | -1, repeat: RepeatMode): Found {
  const n = queue.length
  for (let step = 1; step <= n; step++) {
    let i = from + dir * step
    if (i < 0 || i >= n) {
      if (repeat !== "all") return { kind: "end" }
      i = ((i % n) + n) % n
    }
    const item = queue[i]
    if (item.availability === "online" && !failed.has(item.assetId)) return { kind: "found", index: i }
  }
  return { kind: "none" }
}

export const usePlaybackStore = create<PlaybackState>()((set, get) => {
  /** 항목 선택 — 상태를 먼저 바꾼 뒤 보기 영역을 연다(보기 영역 구독이 현재 항목과 같은 열기는 무시한다) */
  const select = (index: number) => {
    const item = get().queue[index]
    set((s) => ({ index, currentId: item.assetId, status: "playing", token: s.token + 1 }))
    useViewStore.getState().openPreview(item.assetId)
  }
  const giveUp = () => {
    toast.error(i18n.t("playback.noPlayable"))
    get().stop()
  }
  const advance = (dir: 1 | -1) => {
    const s = get()
    if (s.status === "idle") return
    const r = find(s.queue, s.failed, s.index, dir, s.repeat)
    if (r.kind === "found") select(r.index)
    else if (r.kind === "none") giveUp()
    else if (dir === 1) {
      // 반복 끄기: 마지막 항목 뒤 정지 — 보기 영역은 마지막 항목을 그대로 둔다
      set({ status: "stopped" })
    }
  }
  const begin = (from: number) => {
    const s = get()
    const r = find(s.queue, s.failed, from, 1, s.repeat)
    if (r.kind === "found") select(r.index)
    else giveUp()
  }

  return {
    ...IDLE,
    repeat: "off",
    token: 0,
    start: (collectionId, queue, fromAssetId) => {
      // 지정한 항목이 대기열에 없으면 처음부터
      const at = Math.max(0, fromAssetId === undefined ? 0 : queue.findIndex((q) => q.assetId === fromAssetId))
      set({ ...IDLE, collectionId, queue, status: "playing", index: at - 1 })
      begin(at - 1)
    },
    next: () => advance(1),
    prev: () => advance(-1),
    jumpTo: (assetId) => {
      const s = get()
      const at = s.queue.findIndex((q) => q.assetId === assetId)
      if (at < 0) return
      set({ consecutiveFailures: 0 })
      begin(at - 1)
    },
    reportEnded: (assetId) => {
      const s = get()
      if (assetId !== s.currentId || (s.status !== "playing" && s.status !== "paused")) return
      set({ consecutiveFailures: 0 })
      advance(1)
    },
    reportFailed: (assetId) => {
      const s = get()
      if (assetId !== s.currentId || (s.status !== "playing" && s.status !== "paused")) return
      const item = s.queue[s.index]
      const failed = new Set(s.failed).add(assetId)
      const consecutiveFailures = s.consecutiveFailures + 1
      set({ failed, consecutiveFailures })
      toast.warning(i18n.t("playback.skipped", { name: item?.fileName ?? "" }))
      if (consecutiveFailures >= s.queue.length) giveUp()
      else advance(1)
    },
    setQueue: (queue) => {
      const s = get()
      if (s.status === "idle") return
      if (queue.length === 0) return get().stop()
      const at = queue.findIndex((q) => q.assetId === s.currentId)
      // 현재 항목이 빠졌으면 직전 자리로 맞춰, 다음 이동이 그 자리를 이어받은 항목(원래의 다음 항목)이 되게 한다
      set({ queue, index: at >= 0 ? at : Math.min(s.index, queue.length) - 1 })
    },
    toggleRepeat: () => set((s) => ({ repeat: s.repeat === "off" ? "all" : "off" })),
    togglePause: () =>
      set((s) => (s.status === "playing" ? { status: "paused" } : s.status === "paused" ? { status: "playing" } : s)),
    clickAsset: (assetId, mods) => {
      const s = get()
      if (s.status === "idle" || mods.ctrl || mods.shift) return
      // 지금 재생 중인 카드를 다시 눌러도 처음부터 다시 시작하지 않는다
      if (assetId === s.currentId && (s.status === "playing" || s.status === "paused")) return
      if (s.queue.some((q) => q.assetId === assetId)) s.jumpTo(assetId)
    },
    stop: () => set({ ...IDLE }),
  }
})
