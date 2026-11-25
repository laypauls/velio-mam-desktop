import { invoke } from "@tauri-apps/api/core"
import type { RemoteSource, RemoteSourceInput, RemoteTestResult } from "./ipc-contract-remote"

// 원격 스토리지 접속 래퍼 — `ipc.ts` 가 다시 내보낸다. 정본 계약: `ipc-contract-remote.ts`.
// 비밀(`input.secret`)은 인자로만 나가고 이 모듈은 어디에도 남기지 않는다 (로그 없음).

export function listRemoteSources(): Promise<RemoteSource[]> {
  return invoke<RemoteSource[]>("list_remote_sources")
}

export function createRemoteSource(input: RemoteSourceInput): Promise<RemoteSource> {
  return invoke<RemoteSource>("create_remote_source", { input })
}

export function updateRemoteSource(sourceId: number, input: RemoteSourceInput): Promise<RemoteSource> {
  return invoke<RemoteSource>("update_remote_source", { sourceId, input })
}

export function deleteRemoteSource(sourceId: number): Promise<void> {
  return invoke<void>("delete_remote_source", { sourceId })
}

/** sourceId null = 저장 전 시험 (비밀을 input 에 담아야 한다) */
export function testRemoteSource(sourceId: number | null, input: RemoteSourceInput): Promise<RemoteTestResult> {
  return invoke<RemoteTestResult>("test_remote_source", { sourceId, input })
}
