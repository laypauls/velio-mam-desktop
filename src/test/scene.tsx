import { vi } from "vitest"
import type { UiApi } from "@/lib/api"
import type { CutSource, SceneCut, SceneCuts, SceneDetectOptions, SceneState } from "@/lib/ipc-contract"
import { mountHtmlPlayer, type HtmlPlayerOpts } from "./ranges"

/** 서버가 돌려주는 모양(`SceneCuts`, camelCase) 그대로의 컷 목록 */
export function sceneCuts(assetId: number, cuts: SceneCut[], over: Partial<SceneCuts> = {}): SceneCuts {
  return {
    assetId,
    state: cuts.length > 0 ? "done" : "none",
    sensitivity: cuts.some((c) => c.source === "auto") ? "medium" : null,
    error: null,
    cuts,
    suppressedCount: 0,
    updatedAt: 1,
    ...over,
  }
}

export const auto = (ms: number): SceneCut => ({ ms, source: "auto" })
export const user = (ms: number): SceneCut => ({ ms, source: "user" })

/**
 * 장면 command 를 메모리 서버로 바꾼다 — 허용 오차(30fps = 33ms) 안의 add 는 no-op, 자동 컷 remove 는 숨김 개수만 올린다.
 * 실제 mock(`mock-scene.ts`)이 아니라 "Rust 가 돌려줄 값" 을 직접 정해 UI 만 검증하기 위한 것이다.
 */
export function installSceneServer(api: UiApi, assetId: number, initial: SceneCuts) {
  let current = initial
  const handlers = new Set<(id: number) => void>()
  const set = (next: Partial<SceneCuts>) => void (current = { ...current, ...next })
  const get = vi.spyOn(api, "sceneCutsGet").mockImplementation(async () => structuredClone(current))
  const add = vi.spyOn(api, "sceneCutAdd").mockImplementation(async (_id, ms) => {
    if (!current.cuts.some((c) => Math.abs(c.ms - ms) <= 33)) {
      set({ cuts: [...current.cuts, user(ms)].sort((a, b) => a.ms - b.ms), state: current.state === "none" ? "done" : current.state })
    }
    return structuredClone(current)
  })
  const remove = vi.spyOn(api, "sceneCutRemove").mockImplementation(async (_id, ms, source: CutSource) => {
    const at = current.cuts.findIndex((c) => c.source === source && Math.abs(c.ms - ms) <= 33)
    if (at >= 0) set({ cuts: current.cuts.filter((_, i) => i !== at), suppressedCount: current.suppressedCount + (source === "auto" ? 1 : 0) })
    return structuredClone(current)
  })
  const clearAuto = vi.spyOn(api, "sceneCutsClearAuto").mockImplementation(async () => {
    const autos = current.cuts.filter((c) => c.source === "auto").length
    set({ cuts: current.cuts.filter((c) => c.source === "user"), suppressedCount: current.suppressedCount + autos })
    return structuredClone(current)
  })
  const request = vi.spyOn(api, "sceneDetectRequest").mockImplementation(async (ids: number[], _options: SceneDetectOptions) => {
    set({ state: "pending" })
    return { queued: ids.length, skipped: 0 }
  })
  vi.spyOn(api, "onSceneUpdated").mockImplementation(async (h) => {
    handlers.add(h)
    return () => void handlers.delete(h)
  })
  return {
    get,
    add,
    remove,
    clearAuto,
    request,
    /** 서버 상태를 바꾸고 `scene://updated` 를 보낸다 (추출 완료 흉내) */
    complete: (next: Partial<SceneCuts>) => {
      set({ state: "done", ...next })
      for (const h of [...handlers]) h(assetId)
    },
    state: () => current,
    listeners: () => handlers.size,
  }
}

export interface ScenePlayerOpts extends Omit<HtmlPlayerOpts, "setup"> {
  scene?: SceneCuts
}

/** `<video>` 경로 보기 영역 + 장면 메모리 서버 — 컨트롤 바의 장면 UI 를 검증한다 (자산 1, 길이 100초, 레이아웃 폭 1000px) */
export async function mountScenePlayer(opts: ScenePlayerOpts = {}) {
  const initial = opts.scene ?? sceneCuts(1, [])
  let server: ReturnType<typeof installSceneServer> | null = null
  const mounted = await mountHtmlPlayer({
    ...opts,
    setup: (api) => {
      server = installSceneServer(api, 1, initial)
    },
  })
  if (!server) throw new Error("scene server not installed")
  return { ...mounted, scene: server as ReturnType<typeof installSceneServer> }
}

export type { SceneState }
