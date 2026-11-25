/**
 * 내장 플레이어(libmpv 워커 + WebView2 SharedBuffer + WebGL2 캔버스) UI ↔ Rust 코어 계약.
 * 정본: `docs/requests/2026-11/20261119-192800-architect-libmpv-ipc-contract.md` (§1 command, §2 이벤트, §6 TS 경계).
 *
 * 규칙은 `ipc-contract.ts` 와 같다 (snake_case command, camelCase serde, 실패 시 CommandError).
 * 프레임은 이 계약이 아니라 `ipc-player-wire.ts`(SharedBuffer + chrome.webview 메시지)로 전달된다 — Tauri 이벤트로는 보내지 않는다.
 * 모든 시각(ms)은 원본(소스) 타임라인의 정수다. 구간 재생(EDL)의 변환은 워커가 한다.
 */

import type { PlayRange } from "./ipc-contract-ranges"
import type { ReleaseMsg } from "./ipc-player-wire"

/** 캔버스 크기 — 장치 픽셀(CSS px × devicePixelRatio), 각 변 1..=16384. 레터박스를 뺀 영상 영역이다 */
export interface CanvasSize {
  width: number
  height: number
}

/**
 * 플레이어 오류 코드 (CommandError.code / `PlayerStatePayload.errorCode`). `audio_device_unavailable` 은 `system_volume_step` 전용.
 * `player_worker_crashed` 는 이벤트 페이로드에만 나온다 (command 오류 아님).
 */
export type PlayerErrorCode =
  | "player_unavailable"
  | "asset_offline"
  | "open_failed"
  | "audio_device_unavailable"
  | "player_timeout"
  | "player_restarting"
  | "player_restart_failed"
  | "player_worker_crashed"

/** 다시 시도하면 될 수 있는 오류 — 타임아웃, 복원 중 프레임 이동 */
export const PLAYER_RETRYABLE: ReadonlySet<PlayerErrorCode> = new Set<PlayerErrorCode>(["player_timeout", "player_restarting"])

export type PlayerControlAction = "play" | "pause" | "toggle" | "stepForward" | "stepBack"

export interface PlayerOpenResult {
  /** 이 열기(또는 재시작)의 세션 번호 — 이벤트 필터에 쓴다 */
  sessionId: number
  /** 원본 전체 길이(ms), 모르면 null (구간 재생 중에도 원본 길이) */
  lengthMs: number | null
  fps: number | null
  state: "playing" | "paused"
  timeMs: number
}

export type PlayerStateName = "opening" | "playing" | "paused" | "ended" | "error" | "restarting" | "restartFailed"

/**
 * `player://state` 페이로드 — 상태 전환은 즉시, 재생 중 시간 갱신은 최대 5Hz, 탐색·프레임 이동 완료 시 교정된 시각.
 * 일시정지 중 주기 갱신은 없다. `timeMs` 는 원본 타임라인 정수.
 * 거르기: `assetId` 가 같고 `sessionId >= ` 최신 `player_open` 응답의 sessionId (첫 응답 전에는 모두 받는다).
 */
export interface PlayerStatePayload {
  sessionId: number
  assetId: number
  state: PlayerStateName
  timeMs: number
  lengthMs: number | null
  fps: number | null
  /** true 일 때만 포함 — 캐시 대기 중 */
  buffering?: boolean
  /** 정제된 영어 메시지 (경로 없음) */
  error?: string
  errorCode?: PlayerErrorCode
  /** `restarting` / `restartFailed` 의 시도 횟수 */
  restart?: { attempt: number; max: number }
}

export const PLAYER_STATE_EVENT = "player://state"

export interface PlayerApi {
  playerAvailable(): Promise<boolean> // player_available — 워커·libmpv 를 찾았고 WebView2 가 SharedBuffer 를 지원하는가
  playerOpen(assetId: number, canvas: CanvasSize, autoplay: boolean, ranges?: PlayRange[] | null, startMs?: number | null): Promise<PlayerOpenResult> // player_open { assetId, autoplay, canvas, ranges?, startMs? } — 첫 프레임이 게시된 뒤 응답. autoplay=false 면 paused(첫 구간 시작 또는 0). ranges(정수 ms)를 주면 열자마자 구간만 재생, 생략/null = 전체. startMs(정수 ms, 이어보기) 는 null/생략이면 보내지 않는다
  playerSetRanges(assetId: number, ranges: PlayRange[] | null): Promise<void> // player_set_ranges { assetId, ranges } — null/[] = 끔
  playerControl(action: PlayerControlAction): Promise<void> // player_control { action } — 프레임 이동 뒤 시각은 state 이벤트가 알려 준다
  playerSeek(ms: number): Promise<void> // player_seek { ms }
  playerSetRate(rate: number): Promise<void> // player_set_rate { rate } 0.1..=16
  playerSetVolume(volume: number, muted: boolean): Promise<void> // player_set_volume { volume: 0..100, muted }
  playerSetRenderSize(width: number, height: number): Promise<void> // player_set_render_size { width, height } — 캔버스 장치 px. 바뀌면 새 에포크
  playerSetRenderActive(active: boolean): Promise<void> // player_set_render_active { active } — false 면 워커가 프레임을 만들지 않는다
  playerReleaseSlots(release: ReleaseMsg): Promise<void> // player_release_slots { release } — 그린 슬롯 반환(에포크당 최대 8슬롯). fire-and-forget, 실패는 호출부가 무시한다
  playerRestart(): Promise<PlayerOpenResult> // player_restart — 재시작 실패(restartFailed) 뒤 다시 시도
  systemVolumeStep(delta: number): Promise<number> // system_volume_step { delta } — Windows 기본 출력 장치 마스터 음량을 상대 변경(delta -1..1, 0.01 단위), 변경 후 음량 0..1 반환. 실패 audio_device_unavailable
  playerClose(): Promise<void> // player_close
  onPlayerState(handler: (p: PlayerStatePayload) => void): Promise<() => void> // event player://state
}
