import {
  AI_DEFAULTS,
  AI_LIMITS,
  type AiApi,
  type AiCliLoginState,
  type AiProvider,
  type AiSettings,
  type AiSettingsPatch,
  type AiUsageRow,
  type AuthMode,
  type AnalysisChange,
  type CliStatus,
  type LocalModel,
  type ModelProgress,
  type ProviderStatus,
} from "@/lib/ipc-contract-ai"
import type { MockCtx } from "./mock-ctx"
import type { AnalysisEvents } from "./types"
import { createMockAnalysis } from "./mock-ai-analysis"
import { dayKey } from "@/lib/format"
import { cmdError } from "./mock-search"

/** mock AI 모듈이 공유하는 상태. 키 값은 저장하지 않는다 — 보관 방식(keyStorage)만 기록한다. */
export interface MockAiShared {
  settings: AiSettings
  providers: Map<AiProvider, ProviderStatus>
  usage: AiUsageRow[]
  /** 동의 철회 시 대기 중인 외부 분석 취소 */
  onRevoke: () => void
  schedule: (fn: () => void, ms: number) => void
  stepMs: number
  /** `ai://analysis` 구독자에게 변경된 자산을 알린다 */
  emitAnalysis: (changes: AnalysisChange[]) => void
}

const PROVIDERS: AiProvider[] = ["openai", "gemini", "claude"]
const EXTERNAL_KINDS: readonly string[] = ["describe", "keywords", "ocr", "summary"]
const MB = 1024 * 1024

function initialModels(clipReady: boolean, e5Ready: boolean): LocalModel[] {
  const m = (modelId: string, name: string, purpose: string, mb: number, license: string, ready = false): LocalModel => ({
    modelId, name, purpose, sizeBytes: mb * MB, license, state: ready ? "ready" : "missing", progress: null, errorCode: null,
  })
  return [
    m("clip-image", "CLIP 이미지 인코더", "clip-image", 352, "MIT", clipReady),
    m("clip-text", "CLIP 다국어 텍스트 인코더", "clip-text", 480, "MIT", clipReady),
    m("e5-small", "multilingual-e5 텍스트", "e5", 118, "MIT", e5Ready),
    m("whisper-base", "Whisper base", "whisper-base", 142, "MIT", true),
    m("whisper-small", "Whisper small", "whisper-small", 466, "MIT"),
  ]
}

const MOCK_CLI_URL = "https://claude.com/cai/oauth/authorize?code=true&client_id=mock"

function seedUsage(now: number): AiUsageRow[] {
  const rows: AiUsageRow[] = []
  for (let back = 0; back < 30; back += 2) {
    const day = dayKey(now - back * 86_400_000)
    const n = 6 + ((back * 7) % 11)
    rows.push({ day, provider: "claude", model: "claude-sonnet", requests: n, failures: back % 6 === 0 ? 1 : 0, inputTokens: n * 1200, outputTokens: n * 180, images: n, authMode: null })
    if (back % 4 === 0) rows.push({ day, provider: "openai", model: "gpt-vision", requests: 3, failures: 0, inputTokens: 3600, outputTokens: 420, images: 3, authMode: null })
  }
  return rows
}

const intIn = (v: number, min: number, max: number) => Number.isInteger(v) && v >= min && v <= max

function validate(patch: AiSettingsPatch): void {
  const bad = (m: string) => cmdError("invalid_input", m)
  const { imageMaxEdge: e, videoFrames: f } = AI_LIMITS
  const tr = patch.transfer
  if (tr?.imageMaxEdge !== undefined && !intIn(tr.imageMaxEdge, e.min, e.max)) throw bad(`imageMaxEdge must be ${e.min}..${e.max}`)
  if (tr?.videoFrames !== undefined && !intIn(tr.videoFrames, f.min, f.max)) throw bad(`videoFrames must be ${f.min}..${f.max}`)
  const h = AI_LIMITS.scheduleHour
  for (const hour of [patch.schedule?.startHour, patch.schedule?.endHour]) {
    if (hour !== undefined && !intIn(hour, h.min, h.max)) throw bad(`hour must be ${h.min}..${h.max}`)
  }
  if (patch.externalKinds?.some((k) => !EXTERNAL_KINDS.includes(k))) throw bad("invalid external kind")
  if (patch.localStt?.model !== undefined && !["base", "small"].includes(patch.localStt.model)) throw bad("invalid stt model")
}

/** P3 AI mock — 동의·공급자(쓰기 전용 키)·로컬 모델·사용량, 분석·유사 검색은 mock-ai-analysis. */
export function createMockAi(ctx: MockCtx): AiApi & AnalysisEvents & { semanticReady(): boolean } {
  const { wait, now, opts } = ctx
  const o = opts.ai ?? {}
  const schedule = o.schedule ?? ((fn: () => void, ms: number) => void setTimeout(fn, ms))
  const consentAt = now()
  const settings: AiSettings = {
    ...structuredClone(AI_DEFAULTS),
    consent: o.consent ? { given: true, at: consentAt } : { given: false, at: null },
    language: "ko",
  }
  const providers = new Map<AiProvider, ProviderStatus>(
    PROVIDERS.map((provider) => [provider, { provider, keyStorage: "none", authMode: "apiKey", subscriptionSupported: provider === "claude", cli: null, model: null, lastCheck: null }]),
  )
  const models = initialModels(o.clipReady === true, o.e5Ready === true)
  const handlers = new Set<(p: ModelProgress) => void>()
  const analysisHandlers = new Set<(c: AnalysisChange[]) => void>()
  const shared: MockAiShared = {
    settings, providers, usage: seedUsage(consentAt), onRevoke: () => undefined, schedule, stepMs: o.stepMs ?? 700,
    emitAnalysis: (changes) => analysisHandlers.forEach((h) => h(structuredClone(changes))),
  }

  const semanticReady = () => models.filter((m) => m.purpose.startsWith("e5")).every((m) => m.state === "ready")
  const emit = (m: LocalModel, downloaded: number) =>
    handlers.forEach((h) => h({ modelId: m.modelId, state: m.state, downloaded, total: m.sizeBytes }))
  const provider = (p: AiProvider): ProviderStatus => {
    const s = providers.get(p)
    if (!s) throw cmdError("invalid_input", "unknown provider")
    return s
  }
  const snapshot = () => structuredClone(settings)

  // 구독(Claude Code CLI) mock — 설치·로그인 상태와 로그인 세션 하나
  const cliOpt = o.cli ?? {}
  const cli = {
    found: cliOpt.found ?? true,
    supported: cliOpt.supported ?? true,
    loggedIn: cliOpt.loggedIn ?? false,
    unknown: cliOpt.accountUnknown ?? false,
    apiKeyEnv: cliOpt.apiKeyEnv ?? false,
  }
  let login: { pollsLeft: number } | null = null
  const cliStatus = (): CliStatus => ({
    found: cli.found,
    version: cli.found ? "2.1.318" : null,
    supported: cli.found && cli.supported,
    account: !cli.found || cli.unknown ? null : cli.loggedIn ? { loggedIn: true, email: "user@example.com", plan: "max", authMethod: "claude.ai" } : { loggedIn: false },
    apiKeyEnv: cli.apiKeyEnv,
  })
  const view = (s: ProviderStatus): ProviderStatus => ({
    ...structuredClone(s),
    cli: s.subscriptionSupported && s.authMode === "subscription" ? cliStatus() : null,
  })
  const claudeOnly = (p: AiProvider): ProviderStatus => {
    const s = provider(p)
    if (!s.subscriptionSupported) throw cmdError("invalid_input", "subscription not supported")
    return s
  }

  return {
    ...createMockAnalysis(ctx, shared),
    semanticReady,
    getAiSettings: async () => (await wait(), snapshot()),
    updateAiSettings: async (patch) => {
      await wait()
      validate(patch)
      if ((patch.autoAnalyzeNew || patch.backgroundAnalyze) && !settings.consent.given) throw cmdError("consent_required", "consent required")
      if (patch.semanticSearch?.enabled && !semanticReady()) throw cmdError("invalid_input", "local models not ready")
      const { schedule: sch, transfer, localStt, semanticSearch, ...rest } = patch
      Object.assign(settings, rest)
      if (sch) Object.assign(settings.schedule, sch)
      if (transfer) Object.assign(settings.transfer, transfer)
      if (localStt) Object.assign(settings.localStt, localStt)
      if (semanticSearch) Object.assign(settings.semanticSearch, semanticSearch)
      return snapshot()
    },
    giveAiConsent: async () => {
      await wait()
      settings.consent = { given: true, at: now() }
      return snapshot()
    },
    revokeAiConsent: async () => {
      await wait()
      settings.consent = { given: false, at: null }
      settings.autoAnalyzeNew = false
      settings.backgroundAnalyze = false
      shared.onRevoke()
      return snapshot()
    },

    listAiProviders: async () => (await wait(), [...providers.values()].map(view)),
    setApiKey: async (p, key, persist) => {
      await wait()
      const s = provider(p)
      if (key.trim() === "" || key.length > 512) throw cmdError("invalid_input", "invalid key")
      s.keyStorage = persist ? "persisted" : "session" // 키 값은 버린다
      s.lastCheck = null
      if (settings.defaultProvider === null) settings.defaultProvider = p
      return view(s)
    },
    clearApiKey: async (p) => {
      await wait()
      const s = provider(p)
      s.keyStorage = "none"
      s.lastCheck = null
      return view(s)
    },
    setProviderModel: async (p, model) => {
      await wait()
      const s = provider(p)
      s.model = model && model.trim() ? model.trim() : null
      return view(s)
    },
    checkProvider: async (p) => {
      await wait()
      const s = provider(p)
      const errorCode =
        s.authMode === "subscription"
          ? !cli.found ? "cli_not_found" : !cli.supported ? "cli_unsupported_version" : !cli.loggedIn ? "cli_not_logged_in" : (o.checkErrors?.[p] ?? null)
          : s.keyStorage === "none" ? "auth_failed" : (o.checkErrors?.[p] ?? null)
      s.lastCheck = { ok: errorCode === null, at: now(), errorCode }
      return { ok: errorCode === null, errorCode }
    },

    setProviderAuthMode: async (p, mode: AuthMode) => {
      await wait()
      const s = mode === "subscription" ? claudeOnly(p) : provider(p)
      s.authMode = mode
      s.lastCheck = null
      if (mode === "subscription" && settings.defaultProvider === null) settings.defaultProvider = p
      return view(s)
    },
    detectAiCli: async (p) => (await wait(), view(claudeOnly(p))),
    aiCliLoginStart: async (p): Promise<AiCliLoginState> => {
      await wait()
      claudeOnly(p)
      if (!cli.found) return { state: "failed", errorCode: "cli_not_found" }
      login = { pollsLeft: o.cli?.loginPolls ?? 2 }
      return { state: "waiting", url: MOCK_CLI_URL }
    },
    aiCliLoginStatus: async (p): Promise<AiCliLoginState> => {
      await wait()
      claudeOnly(p)
      if (!login) return { state: "none" }
      login.pollsLeft--
      if (login.pollsLeft > 0) return { state: "waiting", url: MOCK_CLI_URL }
      login = null
      cli.loggedIn = true
      cli.unknown = false
      return { state: "done", verified: true }
    },
    aiCliLoginCancel: async (p): Promise<AiCliLoginState> => {
      await wait()
      claudeOnly(p)
      login = null
      return { state: "none" }
    },
    openAiCliLoginUrl: async (p) => (await wait(), claudeOnly(p), login !== null),
    aiCliLogout: async (p) => {
      await wait()
      const s = claudeOnly(p)
      if (o.cli?.busy) throw cmdError("ai_busy", "external ai request in progress")
      cli.loggedIn = false
      cli.unknown = false
      return view(s)
    },

    listLocalModels: async () => (await wait(), models.map((m) => ({ ...m }))),
    downloadModel: async (modelId) => {
      await wait()
      const m = models.find((x) => x.modelId === modelId)
      if (!m) throw cmdError("not_found", "model not found")
      if (m.state === "downloading" || m.state === "ready") return
      m.state = "downloading"
      m.progress = 0
      m.errorCode = null
      emit(m, 0)
      const steps = 4
      for (let i = 1; i <= steps; i++) {
        schedule(() => {
          if (o.downloadError && i === 2) {
            m.state = "failed"
            m.progress = null
            m.errorCode = o.downloadError
            emit(m, Math.round((m.sizeBytes * 1) / steps))
            return
          }
          if (m.state !== "downloading") return
          m.progress = i / steps
          if (i === steps) {
            m.state = "ready"
            m.progress = null
          }
          emit(m, Math.round((m.sizeBytes * i) / steps))
        }, shared.stepMs * i)
      }
    },
    onAnalysisChanged: async (handler) => {
      analysisHandlers.add(handler)
      return () => void analysisHandlers.delete(handler)
    },
    onModelsProgress: async (handler) => {
      handlers.add(handler)
      return () => void handlers.delete(handler)
    },

    listAiUsage: async (fromDay, toDay) => {
      await wait()
      const re = /^\d{4}-\d{2}-\d{2}$/
      if (!re.test(fromDay) || !re.test(toDay) || fromDay > toDay) throw cmdError("invalid_input", "invalid day range")
      return shared.usage.filter((r) => r.day >= fromDay && r.day <= toDay).sort((a, b) => (a.day < b.day ? 1 : a.day > b.day ? -1 : 0)).map((r) => ({ ...r }))
    },
  }
}

