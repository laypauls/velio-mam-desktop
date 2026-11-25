import type { MamApi, ResumeApi, ResumePosition } from "@/lib/ipc-contract"
import type { MockCtx } from "./mock-ctx"
import { cmdError } from "./mock-search"

/** 처음 이 시간 미만은 기록하지 않는다 (D3) */
const MIN_POSITION_MS = 5000
/** 끝에서 이 비율/시간 이내면 '다 봄' (D3) — 둘 중 큰 쪽 */
const END_RATIO = 0.05
const END_MIN_MS = 15_000

/**
 * 이어보기 mock — 메모리에만 저장한다. 저장 판정은 서버(`resume_save`)와 같은 순서:
 * 사용 안 함 → skipped / 길이 없음·최소 길이 미만 → 기록 삭제 + skipped / 처음 5초 → 삭제 + cleared / 끝 5%·15초 이내 → 삭제 + cleared / 그 외 upsert.
 */
export function createMockResume({ settings, db, rec, wait }: Pick<MockCtx, "settings" | "db" | "rec" | "wait">): Pick<MamApi, keyof ResumeApi> {
  const store = new Map<number, ResumePosition>()
  /** 끈 폴더 id — 없으면 켬(새 폴더 기본값) */
  const disabledFolders = new Set<number>()
  const clearIds = (ids: readonly number[]) => ids.reduce((n, id) => n + (store.delete(id) ? 1 : 0), 0)
  const folderOff = (assetId: number) => {
    const folderId = db().recs[assetId - 1]?.folderId
    return folderId !== undefined && disabledFolders.has(folderId)
  }
  return {
    resumeGet: async (assetId) => {
      await wait()
      rec(assetId)
      if (folderOff(assetId)) return null
      const r = store.get(assetId)
      return r ? { ...r } : null
    },
    resumeSave: async (assetId, positionMs, durationMs) => {
      await wait()
      if (!Number.isInteger(positionMs)) return Promise.reject("invalid type: floating point, expected i64")
      rec(assetId)
      if (positionMs < 0) throw cmdError("invalid_input", "positionMs")
      if (!settings.resumeEnabled || folderOff(assetId)) return "skipped"
      const minMs = settings.resumeMinDurationSec * 1000
      if (durationMs === null || (minMs > 0 && durationMs < minMs)) {
        store.delete(assetId)
        return "skipped"
      }
      if (positionMs < MIN_POSITION_MS || positionMs >= durationMs - Math.max(durationMs * END_RATIO, END_MIN_MS)) {
        store.delete(assetId)
        return "cleared"
      }
      store.set(assetId, { positionMs, durationMs })
      return "saved"
    },
    resumeCountAssets: async (assetIds) => {
      await wait()
      return assetIds.filter((id) => store.has(id)).length
    },
    resumeClearAssets: async (assetIds) => {
      await wait()
      return clearIds(assetIds)
    },
    resumeClearFolder: async (folderId) => {
      await wait()
      if (!db().folders.some((f) => f.folderId === folderId)) throw cmdError("not_found", "folder not found")
      return clearIds([...store.keys()].filter((id) => db().recs[id - 1]?.folderId === folderId))
    },
    resumeClearAll: async () => {
      await wait()
      const n = store.size
      store.clear()
      return n
    },
    resumeStats: async () => {
      await wait()
      const counts = new Map<number, number>()
      for (const id of store.keys()) {
        const folderId = db().recs[id - 1]?.folderId
        if (folderId !== undefined) counts.set(folderId, (counts.get(folderId) ?? 0) + 1)
      }
      return { folders: db().folders.map((f) => ({ folderId: f.folderId, count: counts.get(f.folderId) ?? 0, enabled: !disabledFolders.has(f.folderId) })), total: store.size }
    },
    resumeSetFolderEnabled: async (folderId, enabled) => {
      await wait()
      if (!db().folders.some((f) => f.folderId === folderId)) throw cmdError("not_found", "folder not found")
      if (enabled) disabledFolders.delete(folderId)
      else disabledFolders.add(folderId)
    },
  }
}
