// Categories: Happy (T-ui-ipc-ranges-01, T-ui-ipc-player-ranges-01..05, T-ui-ipc-settings-ranges-01) / Validation (T-ui-ipc-settings-ranges-02, T-ui-ipc-ranges-03) / Business rule (T-ui-ipc-ranges-05) / External (T-ui-ipc-ranges-02, 04, T-ui-ipc-player-ranges-06)
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { beforeEach, describe, expect, it, vi } from "vitest"
import type { UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { createTauriApi } from "@/lib/api/tauri"
import type { SearchFilters } from "@/lib/ipc-contract"
import {
  collectionPlayRangeSummary,
  playRangesGet,
  playRangesSet,
  playerOpen,
  playerSetRanges,
  updateSettings,
  type CanvasSize,
} from "./ipc"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

/** 모의 DB 에서 유형별 자산 id 를 n 개 찾는다 (영상은 최소 3초 이상) */
async function findIds(api: UiApi, type: "video" | "image", n: number): Promise<number[]> {
  const filters: SearchFilters = {
    mediaTypes: [type], includeOffline: false, includeMissing: false, capturedFrom: null, capturedTo: null, sizeMin: null, sizeMax: null,
    durationMinMs: null, durationMaxMs: null, folder: null, ratingMin: null, ratingMax: null, favoritesOnly: false, collectionId: null, tagIds: [],
  }
  const page = await api.searchAssets({ query: "", filters, sort: { field: "name", direction: "asc" }, limit: n, cursor: null })
  return page.items.map((i) => i.assetId)
}

const CANVAS: CanvasSize = { width: 640, height: 360 }

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(listen).mockReset()
})

describe("ipc play ranges commands", () => {
  it("T-ui-ipc-ranges-01 [Happy] get/set/summary use the Rust argument names and return the server shapes", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce([{ startMs: 1000, endMs: 2000 }])
    vi.mocked(invoke).mockResolvedValueOnce([{ startMs: 0, endMs: 5000 }])
    vi.mocked(invoke).mockResolvedValueOnce([{ assetId: 7, rangeCount: 2, totalMs: 4000 }])
    // Act
    const got = await playRangesGet(7)
    const saved = await playRangesSet(7, [{ startMs: 0, endMs: 3000 }, { startMs: 3000, endMs: 5000 }])
    const summary = await collectionPlayRangeSummary(3)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["play_ranges_get", { assetId: 7 }],
      ["play_ranges_set", { assetId: 7, ranges: [{ startMs: 0, endMs: 3000 }, { startMs: 3000, endMs: 5000 }] }],
      ["collection_play_range_summary", { collectionId: 3 }],
    ])
    expect(got).toEqual([{ startMs: 1000, endMs: 2000 }])
    expect(saved).toEqual([{ startMs: 0, endMs: 5000 }])
    expect(summary).toEqual([{ assetId: 7, rangeCount: 2, totalMs: 4000 }])
  })

  it("T-ui-ipc-ranges-02 [External] the Tauri api normalizes structured errors and non-CommandError rejections (fractional ms is rejected before the command)", async () => {
    // Arrange
    const api = createTauriApi()
    vi.mocked(invoke).mockRejectedValueOnce({ code: "invalid_input", message: "ranges" })
    vi.mocked(invoke).mockRejectedValueOnce("invalid type: floating point `1000.5`, expected i64")
    vi.spyOn(console, "error").mockImplementation(() => undefined)
    // Act + Assert
    await expect(api.playRangesSet(1, [{ startMs: 0, endMs: 1 }])).rejects.toEqual({ code: "invalid_input", message: "ranges" })
    await expect(api.playRangesSet(1, [{ startMs: 1000.5, endMs: 2000 }])).rejects.toMatchObject({ code: "internal" })
  })

  it("T-ui-ipc-ranges-03 [Validation] the mock mirrors the server: sort + merge, clamps to length, rejects images / too short / fractional ms / missing assets", async () => {
    // Arrange
    const api = createMockApi({ count: 200 })
    const [video] = await findIds(api, "video", 1)
    const [image] = await findIds(api, "image", 1)
    // Act
    const merged = await api.playRangesSet(video, [
      { startMs: 2000, endMs: 2500 },
      { startMs: 0, endMs: 500 },
      { startMs: 500, endMs: 1000 },
    ])
    // Assert: 정렬 + 맞닿음 병합, 저장 후 조회와 같다
    expect(merged).toEqual([
      { startMs: 0, endMs: 1000 },
      { startMs: 2000, endMs: 2500 },
    ])
    await expect(api.playRangesGet(video)).resolves.toEqual(merged)
    await expect(api.playRangesSet(video, [{ startMs: 10, endMs: 11 }])).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.playRangesSet(video, [{ startMs: 5, endMs: 5 }])).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.playRangesSet(image, [{ startMs: 0, endMs: 1000 }])).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.playRangesSet(video, [{ startMs: 0.5, endMs: 1000 }])).rejects.toEqual(expect.any(String))
    await expect(api.playRangesGet(9_999_999)).rejects.toMatchObject({ code: "not_found" })
    // 실패한 저장은 기존 구간을 바꾸지 않는다
    await expect(api.playRangesGet(video)).resolves.toEqual(merged)
    await expect(api.playRangesSet(video, [])).resolves.toEqual([])
  })

  it("T-ui-ipc-ranges-05 [Business rule] a range that is only too short because of the duration clamp is dropped, not a whole-save rejection; explicitly too short still rejects", async () => {
    // Arrange
    const api = createMockApi({ count: 200 })
    const [video] = await findIds(api, "video", 1)
    const dur = (await api.getAssetDetail(video)).durationMs as number
    // Act: 두 번째 구간은 길이 500ms 로 보냈지만 clamp 후 10ms
    const saved = await api.playRangesSet(video, [
      { startMs: 0, endMs: 1000 },
      { startMs: dur - 10, endMs: dur + 490 },
    ])
    // Assert
    expect(saved).toEqual([{ startMs: 0, endMs: 1000 }])
    await expect(api.playRangesSet(video, [{ startMs: dur - 10, endMs: dur - 9 }])).rejects.toMatchObject({ code: "invalid_input" })
    // 전부 clamp 로 사라지면 빈 목록 (저장 거부가 아님)
    await expect(api.playRangesSet(video, [{ startMs: dur - 10, endMs: dur + 490 }])).resolves.toEqual([])
    await expect(api.playRangesGet(video)).resolves.toEqual([])
  })

  it("T-ui-ipc-ranges-04 [External] collection summary only lists items with ranges and rejects an unknown collection", async () => {
    // Arrange
    const api = createMockApi({ count: 200 })
    const collection = await api.createCollection("c")
    const video = await findIds(api, "video", 2)
    await api.addToCollection(collection.collectionId, video)
    await api.playRangesSet(video[0], [{ startMs: 0, endMs: 1000 }, { startMs: 2000, endMs: 3000 }])
    // Act
    const summary = await api.collectionPlayRangeSummary(collection.collectionId)
    // Assert
    expect(summary).toEqual([{ assetId: video[0], rangeCount: 2, totalMs: 2000 }])
    await expect(api.collectionPlayRangeSummary(424242)).rejects.toMatchObject({ code: "not_found" })
  })
})

describe("ipc player ranges / autoplay", () => {
  it("T-ui-ipc-player-ranges-01 [Happy] player_open always carries autoplay (both values); player_set_ranges sends null for off", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue({ lengthMs: 1 })
    // Act
    await playerOpen(1, CANVAS, false)
    await playerOpen(2, CANVAS, true)
    await playerSetRanges(2, [{ startMs: 0, endMs: 1000 }])
    await playerSetRanges(2, null)
    // Assert
    const calls = vi.mocked(invoke).mock.calls
    expect(calls[0]).toEqual(["player_open", { assetId: 1, autoplay: false, canvas: CANVAS }])
    expect(calls[1]).toEqual(["player_open", { assetId: 2, autoplay: true, canvas: CANVAS }])
    expect(calls[2]).toEqual(["player_set_ranges", { assetId: 2, ranges: [{ startMs: 0, endMs: 1000 }] }])
    expect(calls[3]).toEqual(["player_set_ranges", { assetId: 2, ranges: null }])
    expect(Object.keys(calls[0][1] as object)).toContain("autoplay")
  })

  it("T-ui-ipc-player-ranges-03 [Happy] the Tauri api passes autoplay and ranges through to the wrappers", async () => {
    // Arrange
    const api = createTauriApi()
    vi.mocked(invoke).mockResolvedValue(undefined)
    // Act
    await api.playerOpen(5, CANVAS, true)
    await api.playerSetRanges(5, [])
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["player_open", { assetId: 5, autoplay: true, canvas: CANVAS }],
      ["player_set_ranges", { assetId: 5, ranges: [] }],
    ])
  })
})

describe("ipc player_open ranges (optional)", () => {
  const R = [{ startMs: 1000, endMs: 2000 }]

  it("T-ui-ipc-player-ranges-04 [Happy] player_open carries ranges when given (array), including null, and omits the key when undefined", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue({ lengthMs: 1 })
    // Act
    await playerOpen(1, CANVAS, true, R)
    await playerOpen(2, CANVAS, false, null)
    await playerOpen(3, CANVAS, false)
    // Assert
    const calls = vi.mocked(invoke).mock.calls
    expect(calls[0]).toEqual(["player_open", { assetId: 1, autoplay: true, canvas: CANVAS, ranges: R }])
    expect(calls[1]).toEqual(["player_open", { assetId: 2, autoplay: false, canvas: CANVAS, ranges: null }])
    expect(Object.keys(calls[1][1] as object)).toContain("ranges")
    expect(Object.keys(calls[2][1] as object)).not.toContain("ranges")
  })

  it("T-ui-ipc-player-ranges-05 [Happy] the Tauri api forwards ranges through playerOpen; the mock player accepts the new argument and still reports unavailable", async () => {
    // Arrange
    const tauri = createTauriApi()
    vi.mocked(invoke).mockResolvedValue({ lengthMs: 1 })
    const mock = createMockApi({ count: 5 })
    // Act
    await tauri.playerOpen(5, CANVAS, true, R)
    // Assert
    expect(vi.mocked(invoke).mock.calls[0]).toEqual(["player_open", { assetId: 5, autoplay: true, canvas: CANVAS, ranges: R }])
    await expect(mock.playerOpen(5, CANVAS, true, R)).rejects.toMatchObject({ code: "player_unavailable" })
  })

  it("T-ui-ipc-player-ranges-06 [External] player_open with ranges propagates structured errors", async () => {
    // Arrange
    vi.mocked(invoke).mockRejectedValue({ code: "open_failed", message: "x" })
    // Act + Assert
    await expect(playerOpen(1, CANVAS, true, R)).rejects.toEqual({ code: "open_failed", message: "x" })
  })
})

describe("ipc settings (play ranges)", () => {
  it("T-ui-ipc-settings-ranges-01 [Happy] the 4 new settings fields travel in the patch under their camelCase names", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue({})
    // Act
    await updateSettings({ mediaOpenAutoplay: true, playRangeKeys: "premiere", playRangesOnly: true, collectionPlayRangesOnly: true })
    // Assert
    expect(invoke).toHaveBeenCalledWith("update_settings", {
      patch: { mediaOpenAutoplay: true, playRangeKeys: "premiere", playRangesOnly: true, collectionPlayRangesOnly: true },
    })
  })

  it("T-ui-ipc-settings-ranges-02 [Validation] mock defaults match Rust and invalid values are rejected with invalid_input (nothing stored)", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    // Act + Assert: defaults
    await expect(api.getSettings()).resolves.toMatchObject({ mediaOpenAutoplay: false, playRangeKeys: "brackets", playRangesOnly: false, collectionPlayRangesOnly: false })
    // Act + Assert: rejections
    await expect(api.updateSettings({ playRangeKeys: "Brackets" as never })).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.updateSettings({ mediaOpenAutoplay: 1 as never })).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.getSettings()).resolves.toMatchObject({ playRangeKeys: "brackets", mediaOpenAutoplay: false })
    // Act + Assert: round trip
    await api.updateSettings({ playRangeKeys: "premiere" })
    await expect(api.getSettings()).resolves.toMatchObject({ playRangeKeys: "premiere" })
  })
})
