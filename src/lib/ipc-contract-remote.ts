/**
 * 원격 스토리지 접속 계약 — `docs/requests/2026-11/20261121-085600-multi-scan-modules-from-watch-folder.md` §4, 수행 계획 5.
 * 규칙은 `ipc-contract.ts` 와 같다 (snake_case command, camelCase serde, 실패 시 CommandError). `ipc-contract.ts` 가 다시 내보낸다.
 *
 * 비밀(비밀번호·개인 키)은 **쓰기 전용**이다 — 만들 때·바꿀 때 `secret` 으로 보내고, 읽는 쪽 `RemoteSource` 에는 `hasSecret` 만 있다.
 * 프론트는 비밀을 입력 중인 폼 밖(전역 상태·로그·URL)에 두지 않는다.
 *
 * 오류 코드: 이름 중복 `already_registered`, 쓰는 폴더가 있어 삭제 거부 `busy`, 잘못된 값 `invalid_input`
 * (메시지의 필드 이름: `name`·`host`·`port`·`user`·`pollSec`·`hostKey`·`logDir`·`dbPath`·`pathMap`·`secret`·`authKind`·`kind`), 없는 접속 `not_found`.
 * 접속 시험의 실패(연결·인증·지문)는 오류가 아니라 `RemoteTestResult` 로 온다.
 */

/** 접속 종류 — `ddp_audit` = DDP 감사 로그, `ses_queue` = 리눅스 스토리지의 storage-event-service 큐(감지기 `fanotify_feed`). 종류는 만든 뒤 바꿀 수 없다 */
export type RemoteSourceKind = "ddp_audit" | "ses_queue"

export type RemoteAuthKind = "password" | "key"

/** 스토리지(볼륨) 경로 → 이 PC 경로 변환 규칙 한 줄. `from` 은 `/` 구분 볼륨 경로, `to` 는 절대 경로(드라이브 또는 UNC) */
export interface PathMapping {
  from: string
  to: string
}

/** 저장된 접속 — 비밀은 없다 */
export interface RemoteSource {
  sourceId: number
  kind: RemoteSourceKind
  name: string
  host: string
  port: number
  user: string
  authKind: RemoteAuthKind
  /** 비밀이 저장돼 있다 (내용은 돌려주지 않는다) */
  hasSecret: boolean
  /** 사용자가 확인한 호스트 키 지문 (`SHA256:…`). null 이면 아직 확인 전 — 이 상태로는 감시용 접속을 하지 않는다 */
  hostKey: string | null
  /** `ddp_audit`: 감사 로그 폴더 (null = 기본 `/tmp/avfs-dbase/audit`). `ses_queue` 는 항상 null */
  logDir: string | null
  /** `ses_queue`: 스토리지 호스트의 SES `events.db` 경로. `ddp_audit` 는 항상 null */
  dbPath: string | null
  /** 로그·큐 읽기 간격 (초, 1~3600) */
  pollSec: number
  pathMap: PathMapping[]
  createdAt: number
  updatedAt: number
}

/** 생성·수정·접속 시험 입력. 수정에서 `secret` 이 null/없음이면 저장된 비밀을 그대로 둔다 (생성은 필수) */
export interface RemoteSourceInput {
  kind: RemoteSourceKind
  name: string
  host: string
  port: number
  user: string
  authKind: RemoteAuthKind
  /** 비밀번호 또는 개인 키 PEM 본문(암호가 걸리지 않은 키). 쓰기 전용 */
  secret?: string | null
  hostKey: string | null
  /** `ddp_audit` 만 (`ses_queue` 는 null — 아니면 invalid_input logDir) */
  logDir: string | null
  /** `ses_queue` 필수: 절대 경로, 따옴표·셸 메타 문자·줄바꿈·`..` 금지 (invalid_input dbPath). `ddp_audit` 는 null */
  dbPath: string | null
  pollSec: number
  pathMap: PathMapping[]
}

/** 접속 시험 결과 — 실패도 결과다 */
export interface RemoteTestResult {
  ok: boolean
  /** 실패 사유 번역 키 접미사 (`remote.error.<key>`): connect·auth·bad_key·host_key_mismatch·host_key_unconfirmed·timeout·no_log_dir·no_audit_files·no_sqlite·db_unreadable·exec·closed */
  errorKey: string | null
  /** 서버가 내민 호스트 키 지문 — 확인 전이거나 저장된 것과 다를 때 사용자에게 보여 준다. 지문을 확인하기 전에는 비밀을 보내지 않는다 */
  fingerprint: string | null
  /** 읽을 수 있는 감사 로그 파일 수 (`ddp_audit` 성공 때) */
  fileCount: number | null
  /** 이벤트 큐 행 수 (`ses_queue` 성공 때 — sqlite3 가 있고 DB 를 읽을 수 있다는 뜻) */
  rowCount: number | null
  /** 이벤트 큐의 마지막 id (`ses_queue` 성공 때) */
  maxId: number | null
}

export interface RemoteApi {
  /** list_remote_sources — 이름 순 */
  listRemoteSources(): Promise<RemoteSource[]>
  /** create_remote_source { input } — secret 필수 */
  createRemoteSource(input: RemoteSourceInput): Promise<RemoteSource>
  /** update_remote_source { sourceId, input } — 종류는 바꿀 수 없다(invalid_input kind). 쓰는 폴더가 있으면 새 경로 변환도 그 폴더 루트에 닿아야 한다(invalid_input pathMap) */
  updateRemoteSource(sourceId: number, input: RemoteSourceInput): Promise<RemoteSource>
  /** delete_remote_source { sourceId } — 쓰는 폴더가 있으면 busy */
  deleteRemoteSource(sourceId: number): Promise<void>
  /** test_remote_source { sourceId, input } — 네트워크를 쓴다. sourceId 가 있고 input.secret 이 없으면 저장된 비밀을 쓴다. hostKey 가 없으면 인증하지 않고 지문만 돌려준다 */
  testRemoteSource(sourceId: number | null, input: RemoteSourceInput): Promise<RemoteTestResult>
}
