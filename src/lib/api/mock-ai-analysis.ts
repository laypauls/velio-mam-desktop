import { MAX_PAGE_LIMIT, type MediaType } from "@/lib/ipc-contract"
import type {
  AiAnalysis,
  AiApi,
  AiProvider,
  AnalysisKind,
  AnalysisRun,
  AnalysisStatus,
  SimilarItem,
  Transcript,
} from "@/lib/ipc-contract-ai"
import type { MockAiShared } from "./mock-ai"
import type { MockCtx } from "./mock-ctx"
import { toSummary } from "./mock-data"
import { dayKey } from "@/lib/format"
import { sampleFor, transcriptFor } from "./mock-ai-samples"
import { decodeCursor, encodeCursor, cmdError } from "./mock-search"

type AnalysisApi = Pick<
  AiApi,
  | "analyzeAssets" | "analyzeAll" | "retryFailedAnalysis" | "getAssetAi" | "updateAnalysis" | "listAnalysisHistory" | "findSimilar"
>

interface AssetAiRec {
  analysis: AiAnalysis | null
  transcript: Transcript | null
  status: Map<AnalysisKind, AnalysisStatus>
  history: AnalysisRun[]
}

const APPLICABLE: Record<MediaType, AnalysisKind[]> = {
  image: ["describe", "keywords", "ocr", "summary", "embedding"],
  video: ["describe", "keywords", "ocr", "summary", "stt", "embedding"],
  audio: ["summary", "stt"],
}
const KINDS: readonly string[] = ["describe", "keywords", "ocr", "stt", "summary", "embedding"]
const isExternal = (k: AnalysisKind) => k !== "stt" && k !== "embedding"
/** mock 의 `analyzeAll` 은 대기열을 이만큼만 채운다 (타이머 폭주 방지) */
const BATCH_CAP = 50
const SIMILAR_TOTAL = 48

/** P3 분석 대기열·결과·편집·이력·유사 미디어 mock. 상태 전이는 주입 가능한 타이머로 진행한다. */
export function createMockAnalysis(ctx: MockCtx, shared: MockAiShared): AnalysisApi {
  const { wait, now, rec, db, opts } = ctx
  const { settings, providers, schedule, stepMs } = shared
  const store = new Map<number, AssetAiRec>()
  let runSeq = 1

  const recOf = (id: number): AssetAiRec => {
    let r = store.get(id)
    if (!r) store.set(id, (r = { analysis: null, transcript: null, status: new Map(), history: [] }))
    return r
  }
  const statusOf = (r: AssetAiRec, kind: AnalysisKind): AnalysisStatus =>
    r.status.get(kind) ?? { kind, state: "none", errorCode: null, updatedAt: null }
  const notify = (id: number) => {
    const r = recOf(id)
    shared.emitAnalysis([{ assetId: id, status: APPLICABLE[rec(id).type].map((k) => ({ ...statusOf(r, k) })) }])
  }
  const setStatus = (r: AssetAiRec, kind: AnalysisKind, state: AnalysisStatus["state"], errorCode: string | null = null) =>
    r.status.set(kind, { kind, state, errorCode, updatedAt: now() })

  const provider = (): AiProvider | null => {
    const has = (p: AiProvider | null) => p !== null && (providers.get(p)?.keyStorage !== "none" || providers.get(p)?.authMode === "subscription")
    if (has(settings.defaultProvider)) return settings.defaultProvider
    return [...providers.keys()].find((p) => has(p)) ?? null
  }

  const addUsage = (p: AiProvider, model: string, images: number) => {
    const day = dayKey(now())
    const authMode = providers.get(p)?.authMode === "subscription" ? "subscription" : null
    let row = shared.usage.find((u) => u.day === day && u.provider === p && u.model === model && u.authMode === authMode)
    if (!row) shared.usage.push((row = { day, provider: p, model, requests: 0, failures: 0, inputTokens: 0, outputTokens: 0, images: 0, authMode }))
    row.requests++
    row.inputTokens += 1200
    row.outputTokens += 180
    row.images += images
  }

  const finish = (id: number, kind: AnalysisKind, retry: boolean) => {
    const r = recOf(id)
    if (statusOf(r, kind).state !== "running") return
    const a = rec(id)
    const external = isExternal(kind)
    const p = provider()
    const code = retry ? undefined : opts.ai?.failKinds?.[kind]
    const run = (state: "done" | "failed", errorCode: string | null): AnalysisRun => ({
      runId: runSeq++, kind, provider: external ? (p ?? "claude") : "local", state, errorCode, attempts: 1, at: now(),
    })
    if (code) {
      setStatus(r, kind, "failed", code)
      r.history.unshift(run("failed", code))
      if (external && p) addUsage(p, providers.get(p)?.model ?? "mock-model", 0)
      notify(id)
      return
    }
    const s = sampleFor(id)
    if (kind === "stt") {
      r.transcript = transcriptFor(id, a.dur, now())
    } else if (kind !== "embedding") {
      const base: AiAnalysis = r.analysis ?? {
        description: "", keywords: [], ocrText: "", summary: "", language: settings.language,
        provider: p ?? "claude", model: "mock-model", analyzedAt: now(), edited: false,
      }
      if (kind === "describe") Object.assign(base, { description: s.description, edited: false })
      if (kind === "keywords") Object.assign(base, { keywords: [...s.keywords], edited: false })
      if (kind === "ocr") base.ocrText = s.ocrText
      if (kind === "summary") base.summary = s.summary
      base.provider = p ?? base.provider
      base.model = (p && providers.get(p)?.model) || "mock-model"
      base.analyzedAt = now()
      r.analysis = base
      if (p) addUsage(p, base.model, a.type === "audio" ? 0 : 1)
    }
    setStatus(r, kind, "done")
    r.history.unshift(run("done", null))
    notify(id)
  }

  const enqueue = (id: number, kinds: AnalysisKind[], retry: boolean) => {
    const r = recOf(id)
    for (const kind of kinds) {
      setStatus(r, kind, "queued")
      schedule(() => {
        if (statusOf(r, kind).state !== "queued") return
        setStatus(r, kind, "running")
        notify(id)
      }, stepMs)
      schedule(() => finish(id, kind, retry), stepMs * 2)
    }
    notify(id)
  }

  shared.onRevoke = () => {
    for (const r of store.values()) {
      for (const [kind, st] of r.status) {
        if (isExternal(kind) && (st.state === "queued" || st.state === "running")) setStatus(r, kind, "none")
      }
    }
  }

  const plan = (ids: number[], chosen: AnalysisKind[], force: boolean, retry: boolean): { queued: number; skipped: number } => {
    const canExternal = provider() !== null
    let queued = 0
    let skipped = 0
    for (const id of ids) {
      const a = rec(id)
      const r = recOf(id)
      const ks = chosen.filter((k) => APPLICABLE[a.type].includes(k)).filter((k) => {
        const st = statusOf(r, k).state
        if (st === "queued" || st === "running") return false
        return force || retry || st !== "done"
      }).filter((k) => !isExternal(k) || canExternal)
      if (ks.length === 0) skipped++
      else {
        queued++
        enqueue(id, ks, retry)
      }
    }
    return { queued, skipped }
  }

  const requireConsent = () => {
    if (!settings.consent.given) throw cmdError("consent_required", "consent required")
  }
  const defaultKinds = (): AnalysisKind[] => [...settings.externalKinds, ...(settings.localStt.enabled ? (["stt"] as const) : [])]

  return {
    analyzeAssets: async (assetIds, kinds, force) => {
      await wait()
      if (assetIds.length === 0 || assetIds.length > 10_000) throw cmdError("invalid_input", "assetIds must be 1..10000")
      if (kinds !== null && (kinds.length === 0 || kinds.some((k) => !KINDS.includes(k)))) throw cmdError("invalid_input", "invalid kinds")
      const chosen = kinds ?? defaultKinds()
      if (chosen.some(isExternal)) requireConsent()
      return plan(assetIds, chosen, force, false)
    },
    analyzeAll: async (onlyUnanalyzed) => {
      await wait()
      requireConsent()
      const ids: number[] = []
      for (const a of db().recs) {
        if (ids.length >= BATCH_CAP) break
        if (a.status !== "active" || a.avail !== "online") continue
        if (onlyUnanalyzed && store.get(a.id)?.analysis) continue
        ids.push(a.id)
      }
      return plan(ids, defaultKinds(), !onlyUnanalyzed, false)
    },
    retryFailedAnalysis: async (assetIds) => {
      await wait()
      const failed: [number, AnalysisKind[]][] = []
      for (const [id, r] of store) {
        if (assetIds && !assetIds.includes(id)) continue
        const ks = [...r.status.values()].filter((s) => s.state === "failed").map((s) => s.kind)
        if (ks.length) failed.push([id, ks])
      }
      if (failed.some(([, ks]) => ks.some(isExternal))) requireConsent()
      let queued = 0
      for (const [id, ks] of failed) queued += plan([id], ks, false, true).queued
      return { queued, skipped: failed.length - queued }
    },
    getAssetAi: async (assetId) => {
      await wait()
      const a = rec(assetId)
      const r = recOf(assetId)
      return {
        analysis: r.analysis ? structuredClone(r.analysis) : null,
        transcript: r.transcript ? structuredClone(r.transcript) : null,
        status: APPLICABLE[a.type].map((k) => ({ ...statusOf(r, k) })),
      }
    },
    updateAnalysis: async (assetId, edit) => {
      await wait()
      rec(assetId)
      const r = recOf(assetId)
      if (edit.description === undefined && edit.keywords === undefined) throw cmdError("invalid_input", "nothing to edit")
      if (edit.description !== undefined && edit.description.length > 4000) throw cmdError("invalid_input", "description too long")
      const kw = edit.keywords?.map((k) => k.trim()).filter(Boolean)
      if (kw && (kw.length > 50 || kw.some((k) => k.length > 100))) throw cmdError("invalid_input", "invalid keywords")
      if (!r.analysis) throw cmdError("not_found", "no analysis")
      if (edit.description !== undefined) r.analysis.description = edit.description
      if (kw) r.analysis.keywords = [...new Set(kw)]
      r.analysis.edited = true
      notify(assetId)
      return structuredClone(r.analysis)
    },
    listAnalysisHistory: async (assetId) => {
      await wait()
      rec(assetId)
      return structuredClone(recOf(assetId).history)
    },
    findSimilar: async (req) => {
      await wait()
      if (!Number.isInteger(req.limit) || req.limit < 1 || req.limit > MAX_PAGE_LIMIT) throw cmdError("invalid_input", "invalid limit")
      const target = rec(req.assetId)
      const offset = req.cursor === null ? 0 : decodeCursor(req.cursor)
      const recs = db().recs
      const minSimilarity = ctx.settings.similarMinSimilarity
      const all: SimilarItem[] = []
      // id 가 가까운 같은 유형 자산을 이웃으로 삼는다 (결정적)
      for (let k = 1; all.length < SIMILAR_TOTAL && k < recs.length; k++) {
        for (const id of [req.assetId - k, req.assetId + k]) {
          const c = recs[id - 1]
          if (!c || c.status !== "active" || c.type !== target.type || all.length >= SIMILAR_TOTAL) continue
          all.push({ ...toSummary(c), similarity: Math.max(0.5, 0.97 - all.length * 0.01) })
        }
      }
      // 백엔드와 같은 규칙: 유사도 내림차순이므로 하한 미만에서 끊는다 (0 = 제한 없음)
      const cut = all.findIndex((x) => x.similarity < minSimilarity - 1e-9)
      if (cut >= 0) all.length = cut
      const items = all.slice(offset, offset + req.limit)
      const end = offset + items.length
      return { items, total: all.length, nextCursor: end < all.length ? encodeCursor(end) : null }
    },
  }
}
