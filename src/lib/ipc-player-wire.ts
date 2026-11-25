/**
 * 내장 플레이어 프레임 전달 와이어 — WebView2 SharedBuffer + `chrome.webview` 메시지.
 * 정본: `docs/requests/2026-11/20261119-192800-architect-libmpv-ipc-contract.md` §3. golden fixture: `src/test/fixtures/player-wire.json`
 * (Rust `mam-player::protocol::frames` 와 같은 파일을 읽는다 — 한쪽만 바꾸면 테스트가 깨진다).
 *
 * 규칙: 모든 메시지는 `mam: "player.v1"` 를 가진 **객체**다. 호스트→JS 는 `chrome.webview` 메시지, JS→호스트 반환(`ReleaseMsg`)은
 * Tauri command `player_release_slots` 로만 보낸다 — `chrome.webview.postMessage`·`window.ipc.postMessage` 는 호스트에 닿지 않거나 Tauri IPC 와 충돌한다 (계약 §4).
 */

export const PLAYER_WIRE = "player.v1"

/** 에포크당 SharedBuffer(슬롯) 수 */
export const SLOT_COUNT = 3
/** 한 `release` 메시지의 슬롯 수 상한 */
export const MAX_RELEASE_SLOTS = 8
/** 렌더 크기(장치 px) 한 변의 상한 — Rust `CanvasSize` 와 같다 */
export const MAX_CANVAS_PX = 16384

/** SharedBuffer 수신 시 `additionalData` — 에포크의 슬롯 하나 */
export interface BufferInfo {
  mam: typeof PLAYER_WIRE
  epoch: number
  slot: number
  count: 3
  size: number
  w: number
  h: number
  /** 64 의 배수, `>= w*4` */
  stride: number
  fmt: "bgr0"
}

/** 호스트 → JS: 슬롯 `slot` 에 새 프레임이 준비됐다 (`seq` 는 에포크 안에서 1 부터 단조 증가) */
export interface FrameMsg {
  mam: typeof PLAYER_WIRE
  k: "frame"
  epoch: number
  slot: number
  seq: number
  w: number
  h: number
  stride: number
  renderDoneMs: number
}

/** JS → 호스트: 다 쓴 슬롯 반환 (1..=8 개) */
export interface ReleaseMsg {
  mam: typeof PLAYER_WIRE
  k: "release"
  epoch: number
  slots: { slot: number; seq: number }[]
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v)
}

function isUint(v: unknown, min = 0, max = Number.MAX_SAFE_INTEGER): v is number {
  return typeof v === "number" && Number.isInteger(v) && v >= min && v <= max
}

function isSlot(v: unknown): v is number {
  return isUint(v, 0, SLOT_COUNT - 1)
}

export function isBufferInfo(v: unknown): v is BufferInfo {
  if (!isRecord(v) || v.mam !== PLAYER_WIRE || v.fmt !== "bgr0" || v.count !== SLOT_COUNT) return false
  if (!isUint(v.epoch, 1) || !isSlot(v.slot)) return false
  if (!isUint(v.w, 1, MAX_CANVAS_PX) || !isUint(v.h, 1, MAX_CANVAS_PX) || !isUint(v.stride, 4) || !isUint(v.size, 1)) return false
  // stride 는 64 의 배수이고 한 행(w*4)을 담아야 하며, 버퍼는 h 행을 모두 담아야 한다 (그리기가 범위를 넘어 읽지 않도록)
  return v.stride % 64 === 0 && v.stride >= v.w * 4 && v.size >= v.stride * v.h
}

export function isFrameMsg(v: unknown): v is FrameMsg {
  if (!isRecord(v) || v.mam !== PLAYER_WIRE || v.k !== "frame") return false
  if (!isUint(v.epoch, 1) || !isSlot(v.slot) || !isUint(v.seq, 1)) return false
  if (!isUint(v.w, 1, MAX_CANVAS_PX) || !isUint(v.h, 1, MAX_CANVAS_PX) || !isUint(v.stride, 4)) return false
  return typeof v.renderDoneMs === "number" && Number.isFinite(v.renderDoneMs)
}

/** 보내기 직전 점검용 — 호스트 핸들러가 받아들이는 모양과 같다 */
export function isReleaseMsg(v: unknown): v is ReleaseMsg {
  if (!isRecord(v) || v.mam !== PLAYER_WIRE || v.k !== "release" || !isUint(v.epoch, 1)) return false
  if (!Array.isArray(v.slots) || v.slots.length < 1 || v.slots.length > MAX_RELEASE_SLOTS) return false
  return v.slots.every((s: unknown) => isRecord(s) && isSlot(s.slot) && isUint(s.seq, 1))
}
