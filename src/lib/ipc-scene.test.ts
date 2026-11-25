// Categories: Happy (T-ui-scene-01a, 01b) / External (01c) / Validation (01d, 01h) / Business rule (01e tolerance, 01f suppression, 01g request·backfill)
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { beforeEach, describe, expect, it, vi } from "vitest"
import type { UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { createTauriApi } from "@/lib/api/tauri"
import type { SceneCuts, SearchFilters } from "@/lib/ipc-contract"
import {
  SCENE_UPDATED_EVENT,
  onSceneUpdated,
  sceneBackfillCount,
  sceneCutAdd,
  sceneCutRemove,
  sceneCutsClearAuto,
  sceneCutsGet,
  sceneDetectBackfill,
  sceneDetectRequest,
} from "./ipc"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

/** 모의 DB 에서 유형별 자산 id 를 n 개 찾는다 */
async function findIds(api: UiApi, type: "video" | "image", n: number): Promise<number[]> {
  const filters: SearchFilters = {
    mediaTypes: [type], includeOffline: false, includeMissing: false, capturedFrom: null, capturedTo: null, sizeMin: null, sizeMax: null,
    durationMinMs: null, durationMaxMs: null, folder: null, ratingMin: null, ratingMax: null, favoritesOnly: false, collectionId: null, tagIds: [],
  }
  const page = await api.searchAssets({ query: "", filters, sort: { field: "name", direction: "asc" }, limit: n, cursor: null })
  return page.items.map((i) => i.assetId)
}

/** Rust 가 돌려주는 모양(camelCase) */
const SAMPLE: SceneCuts = {
  assetId: 7,
  state: "done",
  sensitivity: "medium",
  error: null,
  cuts: [{ ms: 1000, source: "auto" }, { ms: 2500, source: "user" }],
  suppressedCount: 2,
  updatedAt: 1_700_000_000_000,
}

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(listen).mockReset()
})

describe("ipc scene cuts commands", () => {
  it("T-ui-scene-01a [Happy] wrappers use the Rust command and argument names and return the server shapes", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(SAMPLE)
    // Act
    const got = await sceneCutsGet(7)
    await sceneCutAdd(7, 4200)
    await sceneCutRemove(7, 1000, "auto")
    await sceneCutsClearAuto(7)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["scene_cuts_get", { assetId: 7 }],
      ["scene_cut_add", { assetId: 7, ms: 4200 }],
      ["scene_cut_remove", { assetId: 7, ms: 1000, source: "auto" }],
      ["scene_cuts_clear_auto", { assetId: 7 }],
    ])
    expect(got).toEqual(SAMPLE)
  })

  it("T-ui-scene-01b [Happy] request / backfill / count commands and the scene://updated event", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce({ queued: 2, skipped: 1 })
    vi.mocked(invoke).mockResolvedValueOnce({ queued: 1000 })
    vi.mocked(invoke).mockResolvedValueOnce(1234)
    const unlisten = vi.fn()
    let registered: ((e: { payload: { assetId: number } }) => void) | undefined
    vi.mocked(listen).mockImplementation(async (_name, cb) => {
      registered = cb as typeof registered
      return unlisten
    })
    const handler = vi.fn()
    // Act
    const req = await sceneDetectRequest([1, 2, 3], { redetect: true, restoreSuppressed: false, sensitivity: "high" })
    const back = await sceneDetectBackfill()
    const count = await sceneBackfillCount()
    const off = await onSceneUpdated(handler)
    registered?.({ payload: { assetId: 9 } })
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["scene_detect_request", { assetIds: [1, 2, 3], options: { redetect: true, restoreSuppressed: false, sensitivity: "high" } }],
      ["scene_detect_backfill"],
      ["scene_backfill_count"],
    ])
    expect([req, back, count]).toEqual([{ queued: 2, skipped: 1 }, { queued: 1000 }, 1234])
    expect(SCENE_UPDATED_EVENT).toBe("scene://updated")
    expect(vi.mocked(listen).mock.calls[0][0]).toBe("scene://updated")
    expect(handler.mock.calls).toEqual([[9]])
    expect(off).toBe(unlisten)
  })

  it("T-ui-scene-01c [External] the Tauri api passes every scene command through and keeps structured errors", async () => {
    // Arrange
    const api = createTauriApi()
    vi.mocked(invoke).mockResolvedValue(SAMPLE)
    // Act
    await api.sceneCutsGet(1)
    await api.sceneCutAdd(1, 10)
    await api.sceneCutRemove(1, 10, "user")
    await api.sceneCutsClearAuto(1)
    // Assert
    expect(vi.mocked(invoke).mock.calls.map((c) => c[0])).toEqual(["scene_cuts_get", "scene_cut_add", "scene_cut_remove", "scene_cuts_clear_auto"])
    // External: 오류 코드는 CommandError 로 그대로
    vi.mocked(invoke).mockRejectedValueOnce({ code: "invalid_input", message: "mediaType" })
    await expect(api.sceneCutAdd(1, 10)).rejects.toEqual({ code: "invalid_input", message: "mediaType" })
    vi.mocked(invoke).mockRejectedValueOnce({ code: "not_found", message: "asset" })
    await expect(api.sceneCutsGet(99)).rejects.toMatchObject({ code: "not_found" })
  })
})

describe("mock scene server rules", () => {
  /** 요청한 영상이 타이머 없이 `flush` 로 완료되도록 스케줄러를 쥐고 있는다 */
  function mockApi() {
    const queue: (() => void)[] = []
    const api = createMockApi({ count: 200, scene: { schedule: (fn) => void queue.push(fn) } })
    return { api, flush: () => void queue.splice(0).forEach((fn) => fn()) }
  }

  it("T-ui-scene-01d [Validation] no row = state none; add on a row-less video creates state done with sensitivity null; images → mediaType; bad ms / missing asset rejected", async () => {
    // Arrange
    const { api } = mockApi()
    const [video] = await findIds(api, "video", 1)
    const [image] = await findIds(api, "image", 1)
    const dur = (await api.getAssetDetail(video)).durationMs as number
    // Act + Assert: 행 없음
    await expect(api.sceneCutsGet(video)).resolves.toMatchObject({ state: "none", sensitivity: null, cuts: [], suppressedCount: 0, updatedAt: null })
    // 행 없는 영상에 add → done + sensitivity null (FE: "추출" 버튼 유지)
    const added = await api.sceneCutAdd(video, 1500)
    expect(added).toMatchObject({ assetId: video, state: "done", sensitivity: null, cuts: [{ ms: 1500, source: "user" }] })
    // 검증
    await expect(api.sceneCutAdd(image, 1000)).rejects.toMatchObject({ code: "invalid_input", message: "mediaType" })
    await expect(api.sceneCutAdd(video, -1)).rejects.toMatchObject({ code: "invalid_input", message: "ms" })
    await expect(api.sceneCutAdd(video, dur + 1)).rejects.toMatchObject({ code: "invalid_input", message: "ms" })
    await expect(api.sceneCutAdd(video, 1000.5)).rejects.toEqual(expect.any(String))
    await expect(api.sceneCutAdd(9_999_999, 1000)).rejects.toMatchObject({ code: "not_found" })
    await expect(api.sceneCutsGet(9_999_999)).rejects.toMatchObject({ code: "not_found" })
    // 경계: 0 과 길이 끝은 허용, 실패한 add 는 기존 컷을 바꾸지 않는다
    await expect(api.sceneCutAdd(video, 0)).resolves.toMatchObject({ cuts: [{ ms: 0 }, { ms: 1500 }] })
    await expect(api.sceneCutAdd(video, dur)).resolves.toMatchObject({ cuts: [{ ms: 0 }, { ms: 1500 }, { ms: dur }] })
  })

  it("T-ui-scene-01e [Business rule] tolerance ±1 frame (30fps = 33ms): within → no-op, 34ms away → added; remove picks the nearest within tolerance, no-op otherwise", async () => {
    // Arrange
    const { api } = mockApi()
    const [video] = await findIds(api, "video", 1)
    await api.sceneCutAdd(video, 2000)
    // Act + Assert: 허용 오차 경계
    await expect(api.sceneCutAdd(video, 2033)).resolves.toMatchObject({ cuts: [{ ms: 2000 }] })
    await expect(api.sceneCutAdd(video, 1967)).resolves.toMatchObject({ cuts: [{ ms: 2000 }] })
    const wide = await api.sceneCutAdd(video, 2034)
    expect(wide.cuts.map((c) => c.ms)).toEqual([2000, 2034])
    // remove: 허용 오차 밖이면 no-op, 안이면 최근접 1개
    expect((await api.sceneCutRemove(video, 3000, "user")).cuts.map((c) => c.ms)).toEqual([2000, 2034])
    expect((await api.sceneCutRemove(video, 2030, "user")).cuts.map((c) => c.ms)).toEqual([2000])
    // 출처가 다르면 지우지 않는다 (사용자 컷을 auto 로 지울 수 없다)
    await expect(api.sceneCutRemove(video, 2000, "auto")).resolves.toMatchObject({ cuts: [{ ms: 2000 }], suppressedCount: 0 })
    // 멱등: 이미 없는 컷 제거
    await api.sceneCutRemove(video, 2000, "user")
    await expect(api.sceneCutRemove(video, 2000, "user")).resolves.toMatchObject({ state: "done", cuts: [] })
  })

  it("T-ui-scene-01f [Business rule] suppression: removing an auto cut hides it across re-detection unless restoreSuppressed; user cuts survive; clearAuto hides all", async () => {
    // Arrange
    const { api, flush } = mockApi()
    const [video] = await findIds(api, "video", 1)
    await api.sceneDetectRequest([video], {})
    await expect(api.sceneCutsGet(video)).resolves.toMatchObject({ state: "pending" })
    flush()
    const first = await api.sceneCutsGet(video)
    expect(first.state).toBe("done")
    expect(first.sensitivity).toBe("medium") // 설정 기본값
    const autos = first.cuts.filter((c) => c.source === "auto")
    expect(autos.length).toBeGreaterThan(2)
    const victim = autos[1].ms
    await api.sceneCutAdd(video, 1)
    // Act: 자동 컷 하나 제거 → 숨김 목록
    const afterRemove = await api.sceneCutRemove(video, victim, "auto")
    expect(afterRemove.suppressedCount).toBe(1)
    expect(afterRemove.cuts.some((c) => c.ms === victim)).toBe(false)
    // 다시 추출(되살리기 없음) — 지운 컷은 안 나오고 사용자 컷은 유지, 민감도는 요청값
    await api.sceneDetectRequest([video], { redetect: true, sensitivity: "high" })
    flush()
    const redone = await api.sceneCutsGet(video)
    expect(redone.sensitivity).toBe("high")
    expect(redone.cuts.some((c) => c.ms === victim)).toBe(false)
    expect(redone.cuts.some((c) => c.source === "user" && c.ms === 1)).toBe(true)
    expect(redone.suppressedCount).toBe(1)
    // 되살리기
    await api.sceneDetectRequest([video], { redetect: true, restoreSuppressed: true })
    flush()
    const restored = await api.sceneCutsGet(video)
    expect(restored.cuts.some((c) => c.ms === victim && c.source === "auto")).toBe(true)
    expect(restored.suppressedCount).toBe(0)
    // 자동 컷 모두 삭제 → 사용자 컷만 남고 전부 숨김 목록
    const cleared = await api.sceneCutsClearAuto(video)
    expect(cleared.cuts).toEqual([{ ms: 1, source: "user" }])
    expect(cleared.suppressedCount).toBe(autos.length)
  })

  it("T-ui-scene-01g [Business rule] request: only active videos are queued, done rows are skipped unless redetect, 0 / 10,001 ids rejected; backfill queues only row-less videos and the count matches", async () => {
    // Arrange
    const { api, flush } = mockApi()
    const [video, video2] = await findIds(api, "video", 2)
    const [image] = await findIds(api, "image", 1)
    // Act + Assert: 영상만 등록
    await expect(api.sceneDetectRequest([video, image], {})).resolves.toEqual({ queued: 1, skipped: 1 })
    flush()
    // done 은 건너뜀, redetect 면 다시
    await expect(api.sceneDetectRequest([video], {})).resolves.toEqual({ queued: 0, skipped: 1 })
    await expect(api.sceneDetectRequest([video], { redetect: true })).resolves.toEqual({ queued: 1, skipped: 0 })
    // 검증
    await expect(api.sceneDetectRequest([], {})).rejects.toMatchObject({ code: "invalid_input", message: "assetIds" })
    await expect(api.sceneDetectRequest(Array.from({ length: 10_001 }, (_, i) => i + 1), {})).rejects.toMatchObject({ code: "invalid_input", message: "assetIds" })
    // 백필: 행 없는 영상만 (video 는 행이 있다), 개수와 등록 수 일치, 멱등
    const count = await api.sceneBackfillCount()
    expect(count).toBeGreaterThan(0)
    await expect(api.sceneDetectBackfill()).resolves.toEqual({ queued: count })
    await expect(api.sceneBackfillCount()).resolves.toBe(0)
    await expect(api.sceneDetectBackfill()).resolves.toEqual({ queued: 0 })
    await expect(api.sceneCutsGet(video2)).resolves.toMatchObject({ state: "pending" })
  })

  it("T-ui-scene-01h [Validation] settings: defaults (visible on, auto off, medium) and validation of the three keys", async () => {
    // Arrange
    const { api } = mockApi()
    // Act + Assert
    await expect(api.getSettings()).resolves.toMatchObject({ sceneCutsVisible: true, sceneDetectAuto: false, sceneDetectSensitivity: "medium" })
    await expect(api.updateSettings({ sceneDetectSensitivity: "high", sceneDetectAuto: true, sceneCutsVisible: false })).resolves.toMatchObject({
      sceneCutsVisible: false, sceneDetectAuto: true, sceneDetectSensitivity: "high",
    })
    await expect(api.updateSettings({ sceneDetectSensitivity: "max" as never })).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.updateSettings({ sceneDetectAuto: 1 as never })).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.updateSettings({ sceneCutsVisible: "yes" as never })).rejects.toMatchObject({ code: "invalid_input" })
    // 거부된 patch 는 값을 바꾸지 않는다
    await expect(api.getSettings()).resolves.toMatchObject({ sceneCutsVisible: false, sceneDetectAuto: true, sceneDetectSensitivity: "high" })
  })
})
