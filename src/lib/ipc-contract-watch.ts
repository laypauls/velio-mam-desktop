/**
 * 폴더 변경 감지 방식 · 재스캔 정책 계약 — `docs/requests/2026-11/20261120-230330-architect-change-detection-spec.md` §5.2.
 * 규칙은 `ipc-contract.ts` 와 같다 (snake_case command, camelCase serde, 실패 시 CommandError). `ipc-contract.ts` 가 다시 내보낸다.
 */
import type { ManagedFolder } from "./ipc-contract"

/** 스캔이 한 경로를 3분 넘게 붙들고 있다 (`ManagedFolder.scanStall`) — 응답 없는 공유를 읽는 중일 수 있다. 삭제가 아니다 */
export interface ScanStall {
  /** 붙들린 경로 (표시용 절대 경로) */
  atPath: string
  /** 붙들기 시작한 시각 (UTC ms) */
  since: number
}

/**
 * 변경 신호원 상태 (`ManagedFolder.watchHealth`) — `ok` 는 마지막으로 변경 신호를 받은 시각(없으면 null),
 * `degraded` 는 재연결 중·응답 느림·이벤트 넘침 등 (놓친 변경은 재스캔이 보완한다). `reasonKey` 는 완전한 번역 키(`watch.health.overflow` 등 — 접두어 포함)
 */
export type SourceHealth =
  | { state: "ok"; lastSignalAt: number | null }
  | { state: "degraded"; since: number; reasonKey: string }

/** 감지기가 선언한 정수 옵션 한 칸 — UI 는 이 선언만으로 입력 칸을 만든다 (감지기 추가에 UI 변경 불필요) */
export interface DetectorOptionField {
  key: string
  /** `integer` = 범위가 있는 정수, `bool` = 켜기/끄기(min·max·default 는 0/1), `ref` = 다른 레코드(원격 접속)의 ID — 기본값 없이 항상 저장한다 */
  kind: "integer" | "bool" | "ref"
  min: number
  max: number
  default: number
}

/** `list_change_detectors` 항목 — 이 폴더 기준으로 쓸 수 있는지 */
export interface ChangeDetectorInfo {
  id: string
  available: boolean
  /** 사용할 수 없을 때의 사유 키 (`watch.reason.<key>`) */
  reasonKey: string | null
  optionFields: DetectorOptionField[]
  /** 실측·실기 검증 전 감지기 — UI 가 "실험" 으로 표시한다 */
  experimental: boolean
}

/** 감지기별 옵션 (키는 camelCase, 기본값은 저장하지 않는다). 정수 옵션은 number, 공통 옵션 `exclusiveOpenCheck`(복사 완료 확인에 배타적 열기 사용, 기본 true — 끌 때만 false 로 저장)는 boolean */
export type WatchOptions = Record<string, number | boolean>

export type Weekday = "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun"

/** 이 버전 UI 가 편집할 수 있는 트리거 종류 — 그 밖의 kind 는 "알 수 없는 정책" 으로 읽기만 한다 */
export const KNOWN_TRIGGER_KINDS = ["interval", "schedule", "manual"] as const
export type KnownTriggerKind = (typeof KNOWN_TRIGGER_KINDS)[number]

export type RescanTrigger =
  | { kind: "interval"; hours: number }
  | { kind: "schedule"; days: Weekday[]; at: string }
  | { kind: "manual" }

/** 로컬 시각 "HH:MM", 자정 넘김 허용 (start > end), start == end 는 거부 */
export interface TimeWindow {
  start: string
  end: string
}

export interface RescanPolicy {
  trigger: RescanTrigger
  window: TimeWindow | null
  onStart: boolean
}

/** 서버가 돌려주는 저장값 — 알 수 없는 kind·잘못된 값이 그대로 올 수 있다 (읽기 전용) */
export interface StoredRescanPolicy {
  trigger: { kind: string; [k: string]: unknown }
  window?: unknown
  onStart?: unknown
}

export interface WatchApi {
  /** list_change_detectors { folderId } — 등록 순서 */
  listChangeDetectors(folderId: number): Promise<ChangeDetectorInfo[]>
  /** set_folder_watch_mode { folderId, mode, options } — mode null = 자동(공통 옵션 `exclusiveOpenCheck` 만 허용). 저장 후 감시를 다시 건다 */
  setFolderWatchMode(folderId: number, mode: string | null, options: WatchOptions | null): Promise<ManagedFolder>
  /** set_folder_rescan_policy { folderId, policy } — null = 전역 기본으로 되돌림 */
  setFolderRescanPolicy(folderId: number, policy: RescanPolicy | null): Promise<ManagedFolder>
}
