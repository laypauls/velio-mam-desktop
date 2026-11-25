import type { MamApi } from "@/lib/ipc-contract"
import type { MockDb, Rec } from "./mock-data"
import { cmdError } from "./mock-search"

type UserApi = Pick<
  MamApi,
  | "updateAssetTags"
  | "renameTag"
  | "deleteTag"
  | "setMemo"
  | "setAssetTitle"
  | "setFavorite"
  | "setRating"
  | "ratingCounts"
  | "listCollections"
  | "createCollection"
  | "renameCollection"
  | "deleteCollection"
  | "addToCollection"
  | "removeFromCollection"
  | "reorderCollection"
  | "collectionMembership"
  | "collectionPlayQueue"
>

interface Deps {
  db: () => MockDb
  rec: (id: number) => Rec
  touch: (db: MockDb, r: Rec) => void
  wait: () => Promise<void>
  /** 설정 `titleEditEnabled` — 꺼져 있으면 set_asset_title 을 거부한다 (백엔드와 같은 규칙) */
  titleEditEnabled: () => boolean
}

const norm = (s: string) => s.normalize("NFC").trim().toLowerCase()

/** P2-7 사용자 데이터 mock — 태그·메모·즐겨찾기·컬렉션. */
export function createMockUsers({ db, rec, touch, wait, titleEditEnabled }: Deps): UserApi {
  const findTag = (name: string): number | undefined => {
    const n = norm(name)
    for (const [id, t] of db().tags) if (norm(t) === n) return id
    return undefined
  }
  const requireName = (name: string): string => {
    const v = name.normalize("NFC").trim()
    if (!v) throw cmdError("invalid_input", "name required")
    if (v.length > 100) throw cmdError("invalid_input", "name too long")
    return v
  }
  const requireFreeCollectionName = (name: string, exceptId?: number) => {
    for (const [id, c] of db().collections) {
      if (id !== exceptId && norm(c.name) === norm(name)) throw cmdError("already_registered", "collection exists")
    }
  }
  const collection = (id: number) => {
    const c = db().collections.get(id)
    if (!c) throw cmdError("not_found", "collection not found")
    return c
  }
  const count = (members: Set<number>) => {
    let n = 0
    for (const id of members) if (db().recs[id - 1]?.status === "active") n++
    return n
  }
  const view = (id: number) => {
    const c = collection(id)
    return { collectionId: id, name: c.name, assetCount: count(c.members), createdAt: c.createdAt }
  }

  return {
    updateAssetTags: async ({ assetIds, add, remove }) => {
      await wait()
      const names = add.map(requireName)
      const recs = assetIds.map(rec)
      const ids = names.map((n) => {
        let id = findTag(n)
        if (id === undefined) {
          id = db().nextTagId++
          db().tags.set(id, n)
        }
        return id
      })
      for (const r of recs) {
        r.tags = [...r.tags.filter((t) => !remove.includes(t)), ...ids.filter((t) => !r.tags.includes(t))]
        touch(db(), r)
      }
    },
    renameTag: async (tagId, name) => {
      await wait()
      const v = requireName(name)
      if (!db().tags.has(tagId)) throw cmdError("not_found", "tag not found")
      const dup = findTag(v)
      if (dup !== undefined && dup !== tagId) throw cmdError("already_registered", "tag exists")
      db().tags.set(tagId, v)
      db().version++
      return { tagId, name: v, assetCount: db().recs.filter((r) => r.tags.includes(tagId)).length }
    },
    deleteTag: async (tagId) => {
      await wait()
      if (!db().tags.delete(tagId)) throw cmdError("not_found", "tag not found")
      for (const r of db().recs) if (r.tags.includes(tagId)) r.tags = r.tags.filter((t) => t !== tagId)
      db().version++
    },
    setMemo: async (assetId, memo) => {
      await wait()
      const r = rec(assetId)
      r.memo = memo
      touch(db(), r)
    },
    setAssetTitle: async (assetId, title) => {
      await wait()
      const r = rec(assetId)
      if (!titleEditEnabled()) throw cmdError("invalid_input", "title editing disabled")
      const v = title?.trim() ?? ""
      if ([...v].length > 200) throw cmdError("invalid_input", "title too long")
      r.title = v === "" ? null : v
      touch(db(), r)
    },
    setFavorite: async (assetIds, favorite) => {
      await wait()
      for (const r of assetIds.map(rec)) {
        r.fav = favorite
        touch(db(), r)
      }
    },
    setRating: async (assetIds, rating) => {
      await wait()
      if (!Number.isInteger(rating) || rating < 0 || rating > 5) throw cmdError("invalid_input", "rating out of range")
      for (const r of assetIds.map(rec)) {
        if (r.rating === rating) continue
        r.rating = rating
        touch(db(), r)
      }
    },
    ratingCounts: async () => {
      await wait()
      const counts = [0, 0, 0, 0, 0, 0]
      for (const r of db().recs) if (r.status === "active" && r.avail !== "missing") counts[r.rating]++
      return counts
    },
    listCollections: async () => (await wait(), [...db().collections.keys()].map(view)),
    createCollection: async (name) => {
      await wait()
      const v = requireName(name)
      requireFreeCollectionName(v)
      const id = db().nextCollectionId++
      db().collections.set(id, { name: v, createdAt: Date.now(), members: new Set() })
      return view(id)
    },
    renameCollection: async (id, name) => {
      await wait()
      const c = collection(id)
      const v = requireName(name)
      requireFreeCollectionName(v, id)
      c.name = v
      return view(id)
    },
    deleteCollection: async (id) => {
      await wait()
      collection(id)
      db().collections.delete(id)
      db().version++
    },
    addToCollection: async (id, assetIds) => {
      await wait()
      const c = collection(id)
      assetIds.map(rec).forEach((r) => c.members.add(r.id))
      db().version++
    },
    removeFromCollection: async (id, assetIds) => {
      await wait()
      const c = collection(id)
      assetIds.forEach((a) => c.members.delete(a))
      db().version++
    },
    reorderCollection: async (id, assetIds, target) => {
      await wait()
      const c = collection(id)
      if (assetIds.length === 0) throw cmdError("invalid_input", "assetIds")
      if (assetIds.length > 10_000) throw cmdError("invalid_input", "too many assets")
      const moveSet = new Set(assetIds)
      if (![...moveSet].every((a) => c.members.has(a))) throw cmdError("invalid_input", "assetIds")
      if (target.kind === "before" && (moveSet.has(target.assetId) || !c.members.has(target.assetId))) {
        throw cmdError("invalid_input", "beforeAssetId")
      }
      // 현재 순서를 유지한 채 한 덩어리로 옮긴다
      const all = [...c.members]
      const moved = all.filter((a) => moveSet.has(a))
      const rest = all.filter((a) => !moveSet.has(a))
      const at = target.kind === "start" ? 0 : target.kind === "end" ? rest.length : rest.indexOf(target.assetId)
      rest.splice(at, 0, ...moved)
      c.members = new Set(rest)
      db().version++
    },
    collectionMembership: async (assetIds) => {
      await wait()
      if (assetIds.length > 10_000) throw cmdError("invalid_input", "too many assets")
      const ids = new Set(assetIds)
      const out: { collectionId: number; count: number }[] = []
      for (const [collectionId, c] of db().collections) {
        let n = 0
        for (const a of ids) if (c.members.has(a)) n++
        if (n > 0) out.push({ collectionId, count: n })
      }
      return out
    },
    collectionPlayQueue: async (id) => {
      await wait()
      const c = collection(id)
      const out: Awaited<ReturnType<UserApi["collectionPlayQueue"]>> = []
      for (const a of c.members) {
        const r = db().recs[a - 1]
        if (!r || r.status !== "active") continue
        out.push({
          assetId: r.id,
          mediaType: r.type,
          availability: r.avail,
          fileName: r.name,
          title: r.title,
          durationMs: r.dur,
        })
      }
      return out
    },
  }
}
