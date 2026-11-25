import type { BackupInfo, ProtectionStatus, StartupNotice } from "@/lib/ipc-contract"
import type { MockCtx } from "./mock-ctx"
import type { Rec } from "./mock-data"
import { touch } from "./mock-data"
import { cmdError } from "./mock-search"
import type { UiApi } from "./types"

type ProtectApi = Pick<
  UiApi,
  | "getProtectionStatus" | "setPassword" | "changePassword" | "verifyPassword"
  | "deleteOriginals" | "moveOriginals" | "resetManagedData" | "restoreBackup" | "takeStartupNotice"
>

/** 백엔드와 같은 비밀번호 최소 길이 (P2-9 D1) */
const MIN_PASSWORD = 8
/** 면제 시간 0분일 때 인증 후 보호 명령 1회를 허용하는 시간 (백엔드 SINGLE_USE_WINDOW_MS) */
const SINGLE_USE_WINDOW_MS = 60_000

/** P2-9 보호 mock — 비밀번호·면제 시간·원본 삭제/이동·초기화. `onReset` 은 나머지 mock 상태를 비운다. */
/** mock 내부용: 보호 명령 API + 설정 변경(면제 시간 올리기)이 쓰는 인증 확인 */
export type MockProtect = ProtectApi & { requireAuth: () => void }

export function createMockProtect(
  { db, wait, now, settings, opts }: MockCtx,
  onReset: () => void,
  backups: BackupInfo[],
): MockProtect {
  let startupNotice: StartupNotice | null = opts.startupNotice ?? null
  // singleUseUntil: 면제 시간 0분일 때 인증 직후 보호 명령 1회만 허용하는 창 (백엔드 guard.rs 와 같은 규칙)
  const protection = {
    password: opts.password ?? null,
    validUntil: null as number | null,
    singleUseUntil: null as number | null,
  }
  const protectionStatus = (): ProtectionStatus => ({
    passwordSet: protection.password !== null,
    authValidUntil: protection.validUntil !== null && protection.validUntil > now() ? protection.validUntil : null,
  })
  const requireAuth = () => {
    if (protectionStatus().authValidUntil !== null) return
    if (protection.singleUseUntil !== null && protection.singleUseUntil > now()) {
      protection.singleUseUntil = null
      return
    }
    throw cmdError("auth_required", "authentication required")
  }


  const moved = (ids: number[], fn: (r: Rec) => void) => {
    const failed: { assetId: number; message: string }[] = []
    let succeeded = 0
    for (const id of ids) {
      const r = db().recs[id - 1]
      if (!r || r.status === "deleted") failed.push({ assetId: id, message: "not_found" })
      else if (r.avail !== "online") failed.push({ assetId: id, message: "offline" })
      else {
        fn(r)
        touch(db(), r)
        succeeded++
      }
    }
    return { succeeded, failed }
  }


  return {
    requireAuth,
    getProtectionStatus: async () => protectionStatus(),
    setPassword: async (password) => {
      if (protection.password !== null) throw cmdError("password_already_set", "password already set")
      if (password.length < MIN_PASSWORD) throw cmdError("invalid_input", "password too short")
      protection.password = password
      return protectionStatus()
    },
    changePassword: async (current, next) => {
      if (protection.password === null) throw cmdError("password_not_set", "password not set")
      if (current !== protection.password) throw cmdError("password_incorrect", "password incorrect")
      if (next.length < MIN_PASSWORD) throw cmdError("invalid_input", "password too short")
      protection.password = next
      return protectionStatus()
    },
    verifyPassword: async (password) => {
      if (protection.password === null) throw cmdError("password_not_set", "password not set")
      if (password !== protection.password) throw cmdError("password_incorrect", "password incorrect")
      const minutes = settings.authExemptMinutes
      protection.validUntil = minutes > 0 ? now() + minutes * 60_000 : null
      protection.singleUseUntil = minutes > 0 ? null : now() + SINGLE_USE_WINDOW_MS
      return protectionStatus()
    },
    deleteOriginals: async (ids) => {
      await wait()
      requireAuth()
      return moved(ids, (r) => (r.status = "deleted"))
    },
    moveOriginals: async (ids, destDir) => {
      await wait()
      requireAuth()
      if (!destDir.trim()) throw cmdError("invalid_input", "destDir required")
      return moved(ids, (r) => (r.movedTo = destDir))
    },
    resetManagedData: async (input) => {
      await wait()
      if (!input.forgotPassword) {
        if (protection.password === null) throw cmdError("password_not_set", "password not set")
        if (input.password !== protection.password) throw cmdError("password_incorrect", "password incorrect")
      }
      onReset()
      protection.password = null
      protection.validUntil = null
      settings.firstRunCompleted = false
    },
    // 백엔드와 같은 규칙: 비밀번호 설정 시 면제 시간과 무관하게 매번 인증, 복원 전 pre_restore 백업, 보호 설정은 유지
    restoreBackup: async (input) => {
      await wait()
      if (protection.password !== null) {
        if (!input.password) throw cmdError("auth_required", "authentication required")
        if (input.password !== protection.password) throw cmdError("password_incorrect", "password incorrect")
      }
      if (!backups.some((b) => b.fileName === input.fileName)) throw cmdError("not_found", "backup not found")
      backups.unshift({ fileName: `pre-restore-${backups.length + 1}.db`, createdAt: now(), sizeBytes: 1_048_576, kind: "pre_restore" })
    },
    takeStartupNotice: async () => {
      const notice = startupNotice
      startupNotice = null
      return notice
    },
  }
}
