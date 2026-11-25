// Categories: Happy (T-ui-ipc-sub-01) / Validation (T-ui-ipc-sub-02) / External (T-ui-ipc-sub-03)
import { invoke } from "@tauri-apps/api/core"
import { beforeEach, describe, expect, it, vi } from "vitest"
import type { UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { createTauriApi } from "@/lib/api/tauri"
import { playerSetSubtitle, subtitleCues, subtitleTracks, updateSettings } from "./ipc"
import type { SubtitleTrack } from "./ipc-contract"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

beforeEach(() => {
  vi.mocked(invoke).mockReset()
})

const TRACK: SubtitleTrack = { id: "영상.ko.srt", language: "ko", format: "srt", fileName: "영상.ko.srt" }

/** 모의 DB 에서 가용성이 맞는 자산 id */
async function findAsset(api: UiApi, online: boolean): Promise<number> {
  const page = await api.searchAssets({
    query: "",
    filters: {
      mediaTypes: [], includeOffline: true, includeMissing: true, capturedFrom: null, capturedTo: null, sizeMin: null, sizeMax: null,
      durationMinMs: null, durationMaxMs: null, folder: null, ratingMin: null, ratingMax: null, favoritesOnly: false, collectionId: null, tagIds: [],
    },
    sort: { field: "name", direction: "asc" },
    limit: 20,
    cursor: null,
  })
  const hit = page.items.find((i) => (i.availability === "online") === online)
  if (!hit) throw new Error("no matching asset in the mock db")
  return hit.assetId
}

describe("ipc subtitle commands", () => {
  it("T-ui-ipc-sub-01 [Happy] the three wrappers invoke their snake_case commands with camelCase args; the Tauri api forwards them", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce([TRACK]).mockResolvedValueOnce([{ startMs: 0, endMs: 900, text: "a" }]).mockResolvedValue(undefined)
    // Act
    const tracks = await subtitleTracks(7)
    const cues = await subtitleCues(7, "영상.ko.srt")
    await playerSetSubtitle(7, "영상.ko.srt")
    await playerSetSubtitle(7, null)
    const tauri = createTauriApi()
    await tauri.subtitleTracks(8)
    await tauri.subtitleCues(8, "x")
    await tauri.playerSetSubtitle(8, null)
    // Assert
    expect(tracks).toEqual([TRACK])
    expect(cues).toEqual([{ startMs: 0, endMs: 900, text: "a" }])
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["subtitle_tracks", { assetId: 7 }],
      ["subtitle_cues", { assetId: 7, trackId: "영상.ko.srt" }],
      ["player_set_subtitle", { assetId: 7, trackId: "영상.ko.srt" }],
      ["player_set_subtitle", { assetId: 7, trackId: null }],
      ["subtitle_tracks", { assetId: 8 }],
      ["subtitle_cues", { assetId: 8, trackId: "x" }],
      ["player_set_subtitle", { assetId: 8, trackId: null }],
    ])
    expect(Object.keys(vi.mocked(invoke).mock.calls[3][1] as object)).toContain("trackId") // null 도 키를 보낸다
  })

  it("T-ui-ipc-sub-02 [Validation] the Settings fields travel in the patch; the mock defaults to visible/ko and rejects bad values without storing them", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue({})
    const api = createMockApi({ count: 5 })
    // Act + Assert: patch 이름
    await updateSettings({ subtitleVisible: false, subtitleDefaultLanguage: null })
    expect(invoke).toHaveBeenCalledWith("update_settings", { patch: { subtitleVisible: false, subtitleDefaultLanguage: null } })
    // Act + Assert: 기본값
    await expect(api.getSettings()).resolves.toMatchObject({ subtitleVisible: true, subtitleDefaultLanguage: "ko" })
    // Act + Assert: 거부
    await expect(api.updateSettings({ subtitleDefaultLanguage: "KO" as never })).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.updateSettings({ subtitleDefaultLanguage: "fr" as never })).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.updateSettings({ subtitleVisible: 1 as never })).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.getSettings()).resolves.toMatchObject({ subtitleVisible: true, subtitleDefaultLanguage: "ko" })
    // Act + Assert: 왕복
    await api.updateSettings({ subtitleDefaultLanguage: null, subtitleVisible: false })
    await expect(api.getSettings()).resolves.toMatchObject({ subtitleVisible: false, subtitleDefaultLanguage: null })
    await api.updateSettings({ subtitleDefaultLanguage: "ja" })
    await expect(api.getSettings()).resolves.toMatchObject({ subtitleDefaultLanguage: "ja" })
  })

  it("T-ui-ipc-sub-03 [External] the mock reports no tracks by default, asset_offline for offline assets, invalid_input for bad ids and not_found for unknown tracks", async () => {
    // Arrange
    const probe = createMockApi({ count: 20 })
    const online = await findAsset(probe, true)
    const offline = await findAsset(probe, false)
    const api = createMockApi({ count: 20, subtitles: { [online]: [{ track: TRACK, cues: [{ startMs: 0, endMs: 500, text: "hi" }] }] } })
    // Act + Assert
    await expect(api.subtitleTracks(online)).resolves.toEqual([TRACK])
    await expect(api.subtitleCues(online, TRACK.id)).resolves.toEqual([{ startMs: 0, endMs: 500, text: "hi" }])
    await expect(api.subtitleCues(online, "")).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.subtitleCues(online, "x".repeat(513))).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.subtitleCues(online, "😀".repeat(513))).rejects.toMatchObject({ code: "invalid_input" }) // 글자 수 기준
    await expect(api.subtitleCues(online, "😀".repeat(512))).rejects.toMatchObject({ code: "not_found" }) // UTF-16 길이 1024 여도 512자는 허용
    await expect(api.subtitleTracks(9999)).rejects.toMatchObject({ code: "asset_offline" }) // 자산 없음 → asset_offline (Rust 매핑)
    await expect(api.subtitleCues(9999, "x")).rejects.toMatchObject({ code: "asset_offline" })
    await expect(api.subtitleCues(online, "없음")).rejects.toMatchObject({ code: "not_found" })
    await expect(api.playerSetSubtitle(online, "없음")).rejects.toMatchObject({ code: "not_found" })
    await expect(api.playerSetSubtitle(online, null)).resolves.toBeUndefined()
    await expect(api.subtitleTracks(offline)).rejects.toMatchObject({ code: "asset_offline" })
    await expect(createMockApi({ count: 20 }).subtitleTracks(online)).resolves.toEqual([])
  })
})
