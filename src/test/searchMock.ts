import { vi } from "vitest"
import type { UiApi } from "@/lib/api"
import { monthKey } from "@/lib/format"
import type { AssetSummary, SearchIndex, SearchIndexRequest, SearchPage } from "@/lib/ipc-contract"

interface Options {
  /** 숫자 또는 테스트가 바꿀 수 있는 `{ value }` (색인 세대 변경 시나리오) */
  generation?: number | { value: number }
  totalIsEstimate?: boolean
}

/**
 * `searchAssets`/`searchIndex` 를 고정 목록으로 대체한다 (Rust 의 응답 모양과 같다: prevCursor·startOffset·generation).
 * 커서는 `n:<위치>`, `at`·backward 를 지원한다. 정렬 필드가 촬영일/수정일이면 월 구간, 아니면 단일 구간 "all".
 */
export function mockFixedResults(api: UiApi, source: readonly AssetSummary[] | ((req: SearchIndexRequest) => readonly AssetSummary[]), opts: Options = {}) {
  const gen = () => (typeof opts.generation === "object" ? opts.generation.value : (opts.generation ?? 1))
  const itemsFor = typeof source === "function" ? source : () => source
  const search = vi.spyOn(api, "searchAssets").mockImplementation(async (req) => {
    const items = itemsFor(req)
    const total = items.length
    const cursorAt = req.cursor?.startsWith("n:") ? Number(req.cursor.slice(2)) : null
    let start: number
    if (req.at !== undefined) start = req.at
    else if (req.direction === "backward") start = Math.max(0, (cursorAt ?? 0) - req.limit)
    else start = cursorAt ?? 0
    const end = req.direction === "backward" && cursorAt !== null ? cursorAt : Math.min(total, start + req.limit)
    const page = start >= total ? [] : items.slice(start, end)
    return {
      items: page,
      total,
      nextCursor: page.length > 0 && start + page.length < total ? `n:${start + page.length}` : null,
      prevCursor: page.length > 0 && start > 0 ? `n:${start}` : null,
      startOffset: start >= total ? total : start,
      generation: gen(),
      totalIsEstimate: opts.totalIsEstimate ?? false,
      tookMs: 0,
    } satisfies SearchPage
  })
  const index = vi.spyOn(api, "searchIndex").mockImplementation(async (req) => {
    const field = req.sort.field
    const items = itemsFor(req)
    const total = items.length
    if (field !== "capturedAt" && field !== "modifiedAt") {
      return { total, generation: gen(), kind: "none", sections: total > 0 ? [{ key: "all", label: null, count: total }] : [] } satisfies SearchIndex
    }
    const sections: SearchIndex["sections"] = []
    for (const a of items) {
      const ms = a[field]
      const key = ms === null ? "none" : monthKey(ms)
      const last = sections[sections.length - 1]
      if (last?.key === key) last.count++
      else sections.push({ key, label: null, count: 1 })
    }
    return { total, generation: gen(), kind: "month", sections } satisfies SearchIndex
  })
  return { search, index }
}

/** 월을 가진 가짜 자산 — 같은 달(`month` 0-기준)에 `count` 건 */
export function fakeAssets(count: number, firstId = 1, month = 9, year = 2026): AssetSummary[] {
  return Array.from({ length: count }, (_, i) => ({
    assetId: firstId + i,
    fileName: `file_${firstId + i}.jpg`,
    mediaType: "image" as const,
    availability: "online" as const,
    width: 100,
    height: 100,
    durationMs: null,
    fileSize: 1000,
    capturedAt: Date.UTC(year, month, 15),
    modifiedAt: Date.UTC(year, month, 15),
    favorite: false,
    rating: 0,
    thumbState: "done" as const,
    aiState: "none" as const,
    title: null,
    revision: 1,
  }))
}
