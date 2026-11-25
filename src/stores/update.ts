import { create } from "zustand"
import type { UpdateCheck } from "@/lib/ipc-contract"

export type UpdatePhase = "idle" | "downloading" | "installing"

interface UpdateState {
  /** 직전 확인에서 찾은 업데이트 (없으면 null) */
  available: UpdateCheck | null
  /** 확인이 끝났고 업데이트가 없는 상태 (설정 탭 "최신 버전" 표시용) */
  upToDate: boolean
  checking: boolean
  /** 수동 확인 실패 코드 (설정 탭 인라인 표시) */
  checkError: string | null
  dialogOpen: boolean
  /** "나중에" 로 미룬 버전 — 이번 세션에서는 다시 토스트하지 않는다 */
  dismissedVersion: string | null
  /** 이미 토스트로 알린 버전 */
  notifiedVersion: string | null
  phase: UpdatePhase
  downloaded: number
  total: number | null
  /** 설치 실패 코드 (`UpdateError.code`) */
  installError: string | null
  set: (patch: Partial<Omit<UpdateState, "set">>) => void
}

/** 세션 단위 업데이트 상태 — 영속하지 않는다. */
export const useUpdateStore = create<UpdateState>()((set) => ({
  available: null,
  upToDate: false,
  checking: false,
  checkError: null,
  dialogOpen: false,
  dismissedVersion: null,
  notifiedVersion: null,
  phase: "idle",
  downloaded: 0,
  total: null,
  installError: null,
  set: (patch) => set(patch),
}))
