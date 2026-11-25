import type { PathMapping, RemoteAuthKind, RemoteSource, RemoteSourceInput, RemoteSourceKind } from "@/lib/ipc-contract"
import { isCommandError } from "@/lib/errors"

/** 입력 중 값 (숫자는 문자열) — 비밀은 이 폼 상태 밖으로 나가지 않는다. */
export interface RemoteDraft {
  kind: RemoteSourceKind
  name: string
  host: string
  port: string
  user: string
  authKind: RemoteAuthKind
  /** 쓰기 전용 — 저장된 비밀은 절대 채우지 않는다 */
  secret: string
  hostKey: string | null
  logDir: string
  dbPath: string
  pollSec: string
  pathMap: PathMapping[]
}

export type RemoteField = "name" | "host" | "port" | "user" | "secret" | "logDir" | "dbPath" | "pollSec" | "pathMap" | "hostKey"
/** 필드 → 번역 키 (`settings.folders.remote.error.<key>`) */
export type RemoteErrors = Partial<Record<RemoteField, string>>

/** 종류별 기본 경로 변환 첫 줄의 스토리지 쪽 경로 */
const DEFAULT_FROM: Record<RemoteSourceKind, string> = { ddp_audit: "/", ses_queue: "/data/incoming" }

export const emptyDraft = (kind: RemoteSourceKind): RemoteDraft => ({
  kind,
  name: "",
  host: "",
  port: "22",
  user: "",
  authKind: "password",
  secret: "",
  hostKey: null,
  logDir: "",
  dbPath: "",
  pollSec: "3",
  pathMap: [{ from: DEFAULT_FROM[kind], to: "" }],
})

export const draftOf = (s: RemoteSource): RemoteDraft => ({
  kind: s.kind,
  name: s.name,
  host: s.host,
  port: String(s.port),
  user: s.user,
  authKind: s.authKind,
  secret: "",
  hostKey: s.hostKey,
  logDir: s.logDir ?? "",
  dbPath: s.dbPath ?? "",
  pollSec: String(s.pollSec),
  pathMap: s.pathMap.map((m) => ({ ...m })),
})

const isInt = (text: string, min: number, max: number) => text.trim() !== "" && Number.isInteger(Number(text)) && Number(text) >= min && Number(text) <= max
/** 드라이브(`F:\`) 또는 UNC(`\\서버\공유`) 절대 경로 */
const isWindowsAbsolute = (p: string) => /^[A-Za-z]:[\\/]/.test(p) || /^\\\\[^\\]+\\[^\\]+/.test(p)

/**
 * 저장 전 입력 점검 — 서버 검증(`store::validate`)의 앞단이다. 비밀은 새 접속이거나 인증 방식이 바뀐 때만 필수이고,
 * `needSecret` 을 끄면 점검하지 않는다 (지문만 알아 보는 첫 시험).
 */
export function validateDraft(d: RemoteDraft, stored: RemoteSource | null, needSecret = true): RemoteErrors {
  const e: RemoteErrors = {}
  if (d.name.trim() === "" || [...d.name.trim()].length > 100) e.name = "name"
  if (d.host === "" || d.host.length > 253 || /\s/.test(d.host)) e.host = "host"
  if (!isInt(d.port, 1, 65535)) e.port = "port"
  if (d.user === "" || d.user.length > 128) e.user = "user"
  if (!isInt(d.pollSec, 1, 3600)) e.pollSec = "pollSec"
  const secretRequired = stored === null || stored.authKind !== d.authKind || !stored.hasSecret
  if (needSecret && secretRequired && d.secret.trim() === "") e.secret = "secret"
  if (d.kind === "ses_queue") {
    if (!d.dbPath.startsWith("/") || d.dbPath.endsWith("/")) e.dbPath = "dbPath"
  } else if (d.logDir !== "" && !d.logDir.startsWith("/")) e.logDir = "logDir"
  if (d.pathMap.length < 1 || d.pathMap.length > 32 || d.pathMap.some((m) => !m.from.startsWith("/") || !isWindowsAbsolute(m.to))) e.pathMap = "pathMap"
  return e
}

/** 입력 → IPC 입력. 비밀이 비어 있으면 null (수정에서 저장된 비밀 유지). */
export function toInput(d: RemoteDraft): RemoteSourceInput {
  return {
    kind: d.kind,
    name: d.name.trim(),
    host: d.host,
    port: Number(d.port),
    user: d.user,
    authKind: d.authKind,
    secret: d.secret === "" ? null : d.secret,
    hostKey: d.hostKey,
    logDir: d.kind === "ddp_audit" && d.logDir !== "" ? d.logDir : null,
    dbPath: d.kind === "ses_queue" ? d.dbPath : null,
    pollSec: Number(d.pollSec),
    pathMap: d.pathMap.map((m) => ({ from: m.from, to: m.to })),
  }
}

/** 서버의 `invalid input: <field>` 메시지에서 필드를 읽는다 (알 수 없으면 null) */
export function fieldOfError(e: unknown): RemoteField | null {
  if (!isCommandError(e) || e.code !== "invalid_input") return null
  const m = /invalid input: (\w+)/.exec(e.message ?? "")
  const field = m?.[1]
  if (field === "authKind") return "secret"
  return field !== undefined && FIELDS.has(field) ? (field as RemoteField) : null
}
const FIELDS: ReadonlySet<string> = new Set(["name", "host", "port", "user", "secret", "logDir", "dbPath", "pollSec", "pathMap", "hostKey"])
