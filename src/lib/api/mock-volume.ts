import type { FolderRemovalProgress, IgnoredVolume, KnownVolume, MamApi, PendingVolume } from "@/lib/ipc-contract"
import type { MockCtx } from "./mock-ctx"
import { driveMountOf } from "@/lib/volumes"
import { cmdError } from "./mock-search"

type VolumeApiSlice = Pick<
  MamApi,
  | "listPendingNewVolumes" | "dismissNewVolume" | "listIgnoredVolumes" | "unignoreVolume" | "onPendingVolumesChanged"
  | "folderRemovalImpact" | "listKnownVolumes" | "forgetVolumeRecord" | "onFolderRemovalProgress"
  | "prepareVolumeEject" | "resumeVolumeWatch"
>

/**
 * 새 디스크 mock — 서버 규칙: 나중에(remember) = 대기에서 빼고 무시 목록에 저장, 나중에(기억 안 함) = 대기에서만 제거,
 * 다시 묻기 = 무시 목록에서 빼고 (연결돼 있던 디스크면) 대기 목록으로. 변경 때마다 대기 목록 전체를 이벤트로 낸다.
 * 기억하는 디스크 = `known`(volumes 행) + 묻지 않음 전용 항목, 기록 지우기는 관리 폴더가 있으면 `volume_has_folders`.
 * 기본은 빈 상태 — 시드는 `MockOptions.volumes`.
 */
export function createMockVolume({ opts, now, wait, db }: MockCtx): VolumeApiSlice {
  let pending: PendingVolume[] = structuredClone(opts.volumes?.pending ?? [])
  let ignored: IgnoredVolume[] = structuredClone(opts.volumes?.ignored ?? [])
  let known: KnownVolume[] = structuredClone(opts.volumes?.known ?? [])
  /** 다시 묻기 때 되살릴 연결 중 디스크 */
  const connected = new Map<string, PendingVolume>(structuredClone(opts.volumes?.connected ?? []).map((v) => [v.volumeKey, v]))
  const handlers = new Set<(volumes: PendingVolume[]) => void>()
  const progressHandlers = new Set<(p: FolderRemovalProgress) => void>()
  const emit = () => handlers.forEach((h) => h(structuredClone(pending)))

  /** volumes 행 + 묻지 않음 항목을 volumeKey 로 병합 */
  const merged = (): KnownVolume[] => {
    const ignoredKeys = new Set(ignored.map((i) => i.volumeKey))
    const rows = known.map((k) => ({ ...k, ignored: ignoredKeys.has(k.volumeKey) }))
    const only = ignored
      .filter((i) => !known.some((k) => k.volumeKey === i.volumeKey))
      .map<KnownVolume>((i) => ({
        volumeKey: i.volumeKey, label: i.label, kind: null, lastMountPoint: i.lastMountPoint,
        availability: null, lastSeenAt: null, folderCount: 0, ignored: true,
      }))
    return structuredClone([...rows, ...only])
  }

  /** 같은 디스크(드라이브 문자) 폴더 전부의 꺼내기 준비 상태를 바꾼다 */
  const setPrepared = (mount: string, prepared: boolean) => {
    db().folders = db().folders.map((f) => (driveMountOf(f.path) === mount ? { ...f, ejectPrepared: prepared } : f))
    db().version++
  }
  const sameDisk = (mount: string) => db().folders.filter((f) => driveMountOf(f.path) === mount)

  return {
    prepareVolumeEject: async (folderId) => {
      await wait()
      const folder = db().folders.find((f) => f.folderId === folderId)
      if (!folder) throw cmdError("not_found", "folder not found")
      const mount = driveMountOf(folder.path)
      if (folder.ejectable === false || folder.volumeKind === "network" || mount === null) throw cmdError("invalid_input", "network folder")
      if (folder.offline && folder.ejectPrepared !== true) throw cmdError("offline", "volume offline")
      setPrepared(mount, true)
      return { mountPoint: mount, folderIds: sameDisk(mount).map((f) => f.folderId) }
    },
    resumeVolumeWatch: async (folderId) => {
      await wait()
      const folder = db().folders.find((f) => f.folderId === folderId)
      if (!folder) throw cmdError("not_found", "folder not found")
      const mount = driveMountOf(folder.path)
      if (mount !== null && folder.ejectPrepared === true) setPrepared(mount, false)
    },
    listPendingNewVolumes: async () => (await wait(), structuredClone(pending)),
    dismissNewVolume: async (volumeKey, remember) => {
      await wait()
      const v = pending.find((p) => p.volumeKey === volumeKey)
      if (!v) throw cmdError("not_found", "volume not pending")
      pending = pending.filter((p) => p.volumeKey !== volumeKey)
      if (remember) {
        connected.set(v.volumeKey, v)
        ignored = [...ignored, { volumeKey: v.volumeKey, label: v.label, lastMountPoint: v.mountPoint, ignoredAt: now() }]
      }
      emit()
    },
    listIgnoredVolumes: async () => (await wait(), structuredClone(ignored)),
    unignoreVolume: async (volumeKey) => {
      await wait()
      if (!ignored.some((i) => i.volumeKey === volumeKey)) throw cmdError("not_found", "volume not ignored")
      ignored = ignored.filter((i) => i.volumeKey !== volumeKey)
      const live = connected.get(volumeKey)
      if (live) {
        pending = [...pending, live]
        emit()
      }
    },
    onPendingVolumesChanged: async (handler) => {
      handlers.add(handler)
      return () => void handlers.delete(handler)
    },
    folderRemovalImpact: async (folderId) => {
      await wait()
      if (!db().folders.some((f) => f.folderId === folderId)) throw cmdError("not_found", "folder not found")
      if (opts.volumes?.removalImpact) return { ...opts.volumes.removalImpact }
      const mediaCount = db().recs.filter((r) => r.folderId === folderId && r.status === "active").length
      return { mediaCount, tagLinks: 0, notes: 0, collectionItems: 0, aiResults: 0 }
    },
    listKnownVolumes: async () => (await wait(), merged()),
    forgetVolumeRecord: async (volumeKey) => {
      await wait()
      const row = merged().find((v) => v.volumeKey === volumeKey)
      if (!row) throw cmdError("not_found", "volume not known")
      if (row.folderCount > 0) throw cmdError("volume_has_folders", "volume has folders")
      known = known.filter((k) => k.volumeKey !== volumeKey)
      ignored = ignored.filter((i) => i.volumeKey !== volumeKey)
    },
    onFolderRemovalProgress: async (handler) => {
      progressHandlers.add(handler)
      return () => void progressHandlers.delete(handler)
    },
  }
}
