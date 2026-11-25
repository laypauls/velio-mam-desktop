import type { RemoteSource, RemoteSourceInput, RemoteTestResult } from "@/lib/ipc-contract"
import type { MockCtx } from "./mock-ctx"
import { cmdError } from "./mock-search"
import type { UiApi } from "./types"

type RemoteApiSlice = Pick<UiApi, "listRemoteSources" | "createRemoteSource" | "updateRemoteSource" | "deleteRemoteSource" | "testRemoteSource">

/** 지문 형식 — `SHA256:` + 43자 base64 (Rust `valid_fingerprint` 와 같다) */
const FINGERPRINT = /^SHA256:[A-Za-z0-9+/]{43}$/
/** 접속 시험이 "읽을 수 있다" 고 돌려주는 모의 SES DB 경로 */
export const MOCK_SES_DB = "/srv/ses/events.db"
/** 접속 시험이 "서버가 내민" 것으로 돌려주는 모의 지문 */
export const MOCK_SERVER_FINGERPRINT = "SHA256:Ig4rZ7Z7ET3zVOTrfedvfKbmxZCYcYRmG9eCLLjzATs"

/** Rust `ses::valid_db_path` 와 같다 — 절대 경로, 제어 문자·셸 메타 문자·따옴표·`..`·끝 `/` 금지, 512자 이하 */
const DB_PATH_FORBIDDEN = /[\p{Cc}'"`$\\;&|<>(){}[\]*?!#~^]/u
function validDbPath(p: string): boolean {
  return p.startsWith("/") && !p.endsWith("/") && p.length <= 512 && !DB_PATH_FORBIDDEN.test(p) && !p.split("/").includes("..")
}

const invalid = (field: string) => cmdError("invalid_input", `invalid input: ${field}`)

/** Rust `store::validate` 를 모사한 입력 검증 — 필드 이름만 알려 준다 */
function validate(i: RemoteSourceInput): void {
  if (i.kind !== "ddp_audit" && i.kind !== "ses_queue") throw invalid("kind")
  if (i.name.trim().length < 1 || i.name.trim().length > 100) throw invalid("name")
  if (i.host.length < 1 || i.host.length > 253 || /\s/.test(i.host)) throw invalid("host")
  if (!Number.isInteger(i.port) || i.port < 1 || i.port > 65535) throw invalid("port")
  if (i.user.length < 1 || i.user.length > 128) throw invalid("user")
  if (!Number.isInteger(i.pollSec) || i.pollSec < 1 || i.pollSec > 3600) throw invalid("pollSec")
  if (i.hostKey !== null && !FINGERPRINT.test(i.hostKey)) throw invalid("hostKey")
  if (i.logDir !== null && !i.logDir.startsWith("/")) throw invalid("logDir")
  // 종류마다 쓰는 필드가 다르다 — Rust `store::validate` 와 같다
  if (i.kind === "ddp_audit") {
    if (i.dbPath !== null) throw invalid("dbPath")
  } else {
    if (i.logDir !== null) throw invalid("logDir")
    if (i.dbPath === null || !validDbPath(i.dbPath)) throw invalid("dbPath")
  }
  const absolute = (p: string) => /^[A-Za-z]:[\\/]/.test(p) || /^\\\\[^\\]+\\[^\\]+/.test(p)
  if (i.pathMap.length < 1 || i.pathMap.length > 32 || !i.pathMap.every((m) => absolute(m.to))) throw invalid("pathMap")
}

/** 원격 접속 mock — 서버 규칙(검증 필드, 이름 중복, 쓰는 폴더가 있으면 삭제 거부)을 모사한다. 비밀은 저장하지 않고 `hasSecret` 만 둔다. */
export function createMockRemote({ db, wait }: MockCtx): RemoteApiSlice {
  let sources: RemoteSource[] = []
  let nextId = 1
  const find = (id: number): RemoteSource => {
    const s = sources.find((x) => x.sourceId === id)
    if (!s) throw cmdError("not_found", "not found")
    return s
  }
  const toSource = (id: number, i: RemoteSourceInput, now: number, created: number): RemoteSource => ({
    sourceId: id,
    kind: i.kind,
    name: i.name.trim(),
    host: i.host,
    port: i.port,
    user: i.user,
    authKind: i.authKind,
    hasSecret: true,
    hostKey: i.hostKey,
    logDir: i.logDir,
    dbPath: i.dbPath,
    pollSec: i.pollSec,
    pathMap: structuredClone(i.pathMap),
    createdAt: created,
    updatedAt: now,
  })
  const nameTaken = (name: string, except?: number) =>
    sources.some((s) => s.sourceId !== except && s.name.toLowerCase() === name.trim().toLowerCase())

  return {
    listRemoteSources: async () => {
      await wait()
      return structuredClone([...sources].sort((a, b) => a.name.localeCompare(b.name)))
    },
    createRemoteSource: async (input) => {
      await wait()
      validate(input)
      if (!input.secret) throw invalid("secret")
      if (nameTaken(input.name)) throw cmdError("already_registered", "already registered")
      const s = toSource(nextId++, input, Date.now(), Date.now())
      sources = [...sources, s]
      return structuredClone(s)
    },
    updateRemoteSource: async (sourceId, input) => {
      await wait()
      const old = find(sourceId)
      validate(input)
      if (input.kind !== old.kind) throw invalid("kind")
      if (nameTaken(input.name, sourceId)) throw cmdError("already_registered", "already registered")
      const s = toSource(sourceId, input, Date.now(), old.createdAt)
      sources = sources.map((x) => (x.sourceId === sourceId ? s : x))
      return structuredClone(s)
    },
    deleteRemoteSource: async (sourceId) => {
      await wait()
      find(sourceId)
      const used = db().folders.some((f) => (f.watchMode === "ddp_audit" || f.watchMode === "fanotify_feed") && f.watchOptions?.source === sourceId)
      if (used) throw cmdError("busy", "remote source is used by a folder")
      sources = sources.filter((s) => s.sourceId !== sourceId)
    },
    testRemoteSource: async (sourceId, input): Promise<RemoteTestResult> => {
      await wait()
      validate(input)
      if (sourceId !== null) find(sourceId)
      // 지문을 확인하기 전에는 인증하지 않고 지문만 알려 준다
      const base = { fingerprint: MOCK_SERVER_FINGERPRINT, fileCount: null, rowCount: null, maxId: null }
      if (input.hostKey === null) return { ...base, ok: false, errorKey: "host_key_unconfirmed" }
      if (input.hostKey !== MOCK_SERVER_FINGERPRINT) return { ...base, ok: false, errorKey: "host_key_mismatch" }
      // 지문을 확인한 뒤에야 비밀을 쓴다 — 저장 전 시험에는 비밀이 있어야 한다 (Rust `test_remote_source`)
      if (!input.secret && sourceId === null) throw invalid("secret")
      // `ses_queue`: 모의 호스트의 DB 는 `/srv/ses/events.db` 하나뿐이다
      if (input.kind === "ses_queue") {
        if (input.dbPath !== MOCK_SES_DB) return { ...base, ok: false, errorKey: "db_unreadable" }
        return { ...base, ok: true, errorKey: null, rowCount: 3, maxId: 3 }
      }
      return { ...base, ok: true, errorKey: null, fileCount: 3 }
    },
  }
}
