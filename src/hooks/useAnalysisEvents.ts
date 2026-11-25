import { useQueryClient, type QueryClient } from "@tanstack/react-query"
import { useEffect } from "react"
import { useApi } from "@/lib/api"
import type { AnalysisChange, AnalysisState, AssetAiDetail } from "@/lib/ipc-contract-ai"
import { qk } from "./keys"

/** 이벤트가 흐르는 동안(마지막 이벤트 후 이 시간 이내)에는 폴링을 늦춘다 */
export const EVENT_FRESH_MS = 5000
export const POLL_FAST_MS = 1000
export const POLL_SLOW_MS = 10_000
/** 검색 결과 무효화 모으기 — 첫 완료 이벤트 후 이 시간 뒤 한 번만 */
export const SEARCH_INVALIDATE_MS = 1500

let lastEventAt: number | null = null

/** 이벤트 수신 시각 기록 (테스트는 `resetAnalysisEventClock` 으로 초기화) */
export function noteAnalysisEvent(at: number): void {
  lastEventAt = at
}
export function resetAnalysisEventClock(): void {
  lastEventAt = null
}

/**
 * AI 탭 폴링 주기 — 분석 중이고 최근 이벤트가 없을 때만 1초, 그 외(이벤트 정상 / 대기 단계 없음)는 10초.
 * 대량 등록·동의 철회는 이벤트가 없으므로 폴링을 끊지 않는다.
 */
export function analysisPollInterval(busy: boolean, at: number): number {
  const flowing = lastEventAt !== null && at - lastEventAt < EVENT_FRESH_MS
  return busy && !flowing ? POLL_FAST_MS : POLL_SLOW_MS
}

const isTerminal = (s: AnalysisState) => s === "done" || s === "failed"

/** 변경 하나를 캐시에 반영한다. 반환: [상세 재조회 필요, 검색 결과 갱신 필요] */
function applyChange(qc: QueryClient, c: AnalysisChange): [boolean, boolean] {
  const cached = qc.getQueryData<AssetAiDetail>(qk.assetAi(c.assetId))
  const prev = new Map(cached?.status.map((s) => [s.kind, s.state]))
  const reached = c.status.filter((s) => isTerminal(s.state) && prev.get(s.kind) !== s.state)
  if (cached) {
    qc.setQueryData<AssetAiDetail>(qk.assetAi(c.assetId), {
      ...cached,
      status: cached.status.map((s) => c.status.find((n) => n.kind === s.kind) ?? s),
    })
  }
  // 카드 AI 표시(완료·실패)와 검색 문서가 바뀌는 단계 — embedding(자동 백필)은 제외
  const searchChanged = c.status.some(
    (s) =>
      s.kind !== "embedding" &&
      (s.state === "done" || s.state === "failed") &&
      (!cached || prev.get(s.kind) !== s.state),
  )
  return [cached !== undefined && reached.length > 0, searchChanged]
}

/**
 * 앱 전체에서 한 번 구독하는 `ai://analysis` 처리 — 자산별 상태 캐시를 병합하고,
 * 단계가 완료·실패하면 해당 자산의 AI 상세·이력을 다시 읽으며, AI 결과가 생기거나 단계가 실패하면(embedding 제외) 검색 결과를 (모아서) 갱신한다.
 */
export function useAnalysisEvents(): void {
  const api = useApi()
  const qc = useQueryClient()
  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | null = null
    let searchTimer: ReturnType<typeof setTimeout> | null = null
    void api
      .onAnalysisChanged((changes) => {
        noteAnalysisEvent(Date.now())
        let search = false
        for (const c of changes) {
          const [detail, s] = applyChange(qc, c)
          search ||= s
          // 임베딩이 막 완료됐으면 이 자산의 유사 미디어 결과(빈 페이지일 수 있음)를 다시 읽는다
          if (c.status.some((s) => s.kind === "embedding" && s.state === "done")) {
            void qc.invalidateQueries({ queryKey: qk.similar(c.assetId) })
          }
          if (detail) {
            void qc.invalidateQueries({ queryKey: qk.assetAi(c.assetId) })
            void qc.invalidateQueries({ queryKey: qk.analysisHistory(c.assetId) })
          }
        }
        if (search && searchTimer === null) {
          searchTimer = setTimeout(() => {
            searchTimer = null
            void qc.invalidateQueries({ queryKey: qk.search })
          }, SEARCH_INVALIDATE_MS)
        }
      })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
    return () => {
      disposed = true
      unlisten?.()
      if (searchTimer !== null) clearTimeout(searchTimer)
    }
  }, [api, qc])
}
