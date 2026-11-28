/**
 * P3 AI 확장 UI ↔ Rust 코어 계약 초안 (백엔드 세션 검토 대상).
 *
 * 규칙은 `ipc-contract.ts` 와 같다 (snake_case command, camelCase serde, 실패 시 CommandError, 시각은 Unix ms).
 * 제약 (백엔드 합의):
 *  - API 키는 UI 에서 **쓰기 전용**이다 — 설정·삭제·보유 여부만. 키 값은 어떤 응답·이벤트에도 실리지 않는다
 *  - 의미 검색은 `SearchRequest.mode` 옵션이며, UI 가 입력 완료 후 디바운스해 호출한다
 *  - 외부 전송은 사용자 최초 동의(consent) 전에는 일어나지 않는다. 원본 음성은 외부로 보내지 않는다 (자막 생성은 로컬)
 */

import type { AssetSummary, Page, SearchPage } from "./ipc-contract"

// ─── 검색 모드 ──────────────────────────────────────────────────────────────

/** keyword: 풀텍스트만, hybrid: 풀텍스트 + 벡터 후보를 RRF 결합 (임베딩 모델 준비 전에는 keyword 로 처리하고 `degraded` 표시) */
export type SearchMode = "keyword" | "hybrid"

/** `SearchRequest` 에 추가되는 필드. 생략 시 "keyword" */
export interface SearchModeOption {
  mode?: SearchMode
}

/** `SearchPage` 에 추가되는 필드 */
export interface SearchPageAiInfo {
  /** hybrid 요청을 keyword 로 처리했다 (모델 미준비·인덱스 재구축 중 등) */
  degraded?: boolean
}

// ─── 공급자·키·동의 ─────────────────────────────────────────────────────────

export type AiProvider = "openai" | "gemini" | "claude"

/** 키 보관 방식 — persisted: DPAPI 로 보호해 저장, session: 실행 중 메모리만, none: 없음 */
export type KeyStorage = "persisted" | "session" | "none"

/** `list_ai_providers` 항목 — 키 값은 절대 포함하지 않는다 */
/** 공급자 인증 방식 — apiKey: 기존 API 키, subscription: 사용자가 설치한 Claude Code CLI 의 구독 로그인 (claude 만) */
export type AuthMode = "apiKey" | "subscription"

export interface CliAccount {
  loggedIn: boolean
  email?: string
  /** claude auth status 의 subscriptionType (예: "max") */
  plan?: string
  /** "claude.ai" 등 */
  authMethod?: string
}

export interface CliStatus {
  found: boolean
  version: string | null
  /** 최소 지원 버전 이상 */
  supported: boolean
  /** null = 알 수 없음(상태 명령 실패) — 로그아웃으로 단정하지 않는다 */
  account: CliAccount | null
  /** 이 PC 환경변수의 ANTHROPIC_API_KEY/ANTHROPIC_AUTH_TOKEN 이 있다 → CLI 가 구독 대신 API 키로 과금 */
  apiKeyEnv: boolean
}

export type AiCliLoginState =
  | { state: "none" }
  | { state: "waiting"; url: string | null }
  | { state: "done"; verified: boolean }
  | { state: "failed"; errorCode: string }

export interface ProviderStatus {
  provider: AiProvider
  keyStorage: KeyStorage
  /** 저장값 없으면 "apiKey" */
  authMode: AuthMode
  /** 구독 인증을 지원하는 공급자 (claude 만 true) */
  subscriptionSupported: boolean
  /** subscriptionSupported && authMode === "subscription" 일 때만, 그 외 null */
  cli: CliStatus | null
  /** 사용할 모델 ID (설정 가능, null = 공급자 기본) */
  model: string | null
  /** 마지막 연결 확인 결과 */
  lastCheck: { ok: boolean; at: number; errorCode: string | null } | null
}

export interface ProviderCheck {
  ok: boolean
  /** auth_failed | offline | rate_limited | quota_exceeded | unknown */
  errorCode: string | null
}

/**
 * 용어: 화면·프론트 코드에서는 STT 를 "자막(subtitle)"이라 부른다. 단 DB·설정 문서와 1:1 인 저장·교환 값은 호환을 위해 그대로 둔다 —
 * 종류 값 `"stt"`, 설정 키 `localStt`, 응답 필드 `transcript`. 표시 문구는 `analysisKindKey` 가 `ai.kind.subtitle` 로 변환한다.
 */
export type AnalysisKind = "describe" | "keywords" | "ocr" | "stt" | "summary" | "embedding"

export interface TransferScope {
  /** 외부로 보내는 축소 이미지의 긴 변 (px) */
  imageMaxEdge: number
  /** 영상 대표 프레임 수 */
  videoFrames: number
  /** 파일명·촬영일 등 메타데이터를 함께 보낸다 */
  includeMetadata: boolean
}

/** `get_ai_settings` */
export interface AiSettings {
  /** 기본 외부 AI 공급자 (요구사항 §2.5) */
  defaultProvider: AiProvider | null
  /** 최초 외부 전송 동의 */
  consent: { given: boolean; at: number | null }
  /** AI 결과 생성 언어 (설치 언어, 예: "ko") */
  language: string
  /** 신규 등록 자산 자동 분석 */
  autoAnalyzeNew: boolean
  /** 새 파일 자동 자막 생성 — null 은 사용자가 아직 정하지 않음(실효값은 `effectiveAutoSubtitle`). 로컬 처리라 외부 동의와 무관 */
  autoSubtitleNew: boolean | null
  /** 미분석 자산 백그라운드 분석 */
  backgroundAnalyze: boolean
  /** 예약 실행 시간대 (로컬 시각, 시작 == 끝이면 하루 종일) */
  schedule: { enabled: boolean; startHour: number; endHour: number }
  /** 외부 분석 종류 (stt 는 로컬 전용이라 제외) */
  externalKinds: Exclude<AnalysisKind, "stt" | "embedding">[]
  transfer: TransferScope
  /** 로컬 자막 (whisper) — 설정 문서 키는 `localStt` 유지 */
  localStt: { enabled: boolean; model: "base" | "small" }
  /** 로컬 임베딩·의미 검색 */
  semanticSearch: { enabled: boolean }
}

export type AiSettingsPatch = Partial<
  Omit<AiSettings, "consent" | "schedule" | "transfer" | "localStt" | "semanticSearch" | "autoSubtitleNew">
> & {
  schedule?: Partial<AiSettings["schedule"]>
  transfer?: Partial<TransferScope>
  /** null 로 되돌릴 수 없다. 외부 AI 동의가 필요 없다 */
  autoSubtitleNew?: boolean
  localStt?: Partial<AiSettings["localStt"]>
  semanticSearch?: Partial<AiSettings["semanticSearch"]>
}

// ─── 로컬 모델 ──────────────────────────────────────────────────────────────

export type ModelState = "missing" | "downloading" | "ready" | "failed"

/** `list_local_models` 항목 (P1-6 매니페스트) */
export interface LocalModel {
  modelId: string
  name: string
  /** clip-image | clip-text | e5 | whisper-base | whisper-small 등 */
  purpose: string
  sizeBytes: number
  license: string
  state: ModelState
  /** downloading 일 때 0~1 */
  progress: number | null
  errorCode: string | null
}

/** `models://progress` 이벤트 */
export interface ModelProgress {
  modelId: string
  state: ModelState
  downloaded: number
  total: number
}

export const MODELS_PROGRESS_EVENT = "models://progress"

// ─── 분석 결과 (상세 패널) ──────────────────────────────────────────────────

export type AnalysisState = "none" | "queued" | "running" | "done" | "failed" | "skipped"

export interface AnalysisStatus {
  kind: AnalysisKind
  state: AnalysisState
  /** failed 일 때 오류 코드 (auth_failed, offline, unsupported 등) */
  errorCode: string | null
  updatedAt: number | null
}

export interface AiAnalysis {
  description: string
  keywords: string[]
  ocrText: string
  summary: string
  /** 결과 언어 */
  language: string
  provider: AiProvider | "local"
  model: string
  analyzedAt: number
  /** 사용자가 편집했다 (재분석 시 덮어써진다 — 요구사항 §2.5) */
  edited: boolean
}

export interface SubtitleSegment {
  startMs: number
  endMs: number
  text: string
}

export interface Subtitle {
  /** 원문 언어 (감지 결과 보존) */
  language: string
  text: string
  segments: SubtitleSegment[]
  engine: string
  createdAt: number
}

/** `get_asset_detail` 의 `AssetDetail` 에 추가되는 `ai` 필드 */
export interface AssetAiDetail {
  analysis: AiAnalysis | null
  transcript: Subtitle | null
  status: AnalysisStatus[]
}

export interface AnalysisEdit {
  description?: string
  keywords?: string[]
}

/** `list_analysis_history` 항목 — 파일별 성공/실패 이력 */
export interface AnalysisRun {
  runId: number
  kind: AnalysisKind
  provider: AiProvider | "local"
  state: "done" | "failed"
  errorCode: string | null
  attempts: number
  at: number
}

/** `list_ai_usage` 항목 — 일·공급자·모델별 집계 */
export interface AiUsageRow {
  day: string // YYYY-MM-DD (로컬)
  provider: AiProvider
  model: string
  requests: number
  failures: number
  inputTokens: number
  outputTokens: number
  images: number
  /** null = 기존 행 = apiKey */
  authMode: AuthMode | null
}

export interface QueueResult {
  queued: number
  /** 동의 없음·키 없음 등으로 건너뛴 수 */
  skipped: number
}

export interface SimilarRequest {
  assetId: number
  limit: number
  cursor: string | null
}

export interface SimilarItem extends AssetSummary {
  /** 0~1, 높을수록 비슷함 (표시용) */
  similarity: number
}

// ─── 기본값·범위 (백엔드가 같은 값을 강제한다 — 2026-10-28 합의) ───────────────

/** P3 에서 추가되는 오류 코드 — `ErrorCode` 에 합친다 */
export type AiErrorCode =
  /** 동의 전 외부 분석 요청 */
  | "consent_required"
  /** 외부 AI API 4xx/5xx/timeout (메시지는 정제됨, 키 비포함) */
  | "provider_error"
  /** 진행 중 외부 AI 요청이 있어 구독 로그아웃을 거부 */
  | "ai_busy"

export const AI_LIMITS = {
  imageMaxEdge: { min: 256, max: 2048, default: 1024 },
  videoFrames: { min: 1, max: 10, default: 5 },
  scheduleHour: { min: 0, max: 23 },
} as const

/**
 * 기본 설정. includeMetadata=true 여도 절대 경로는 보내지 않는다 (파일명·촬영일·유형·길이와 사용자 메타데이터 필드 값만).
 * 동의 전에는 autoAnalyzeNew/backgroundAnalyze 가 false, 로컬 모델 준비 전에는 semanticSearch 가 false.
 */
export const AI_DEFAULTS: Omit<AiSettings, "consent" | "language"> = {
  defaultProvider: null,
  autoAnalyzeNew: false,
  autoSubtitleNew: null,
  backgroundAnalyze: false,
  schedule: { enabled: false, startHour: 0, endHour: 0 },
  externalKinds: ["describe", "keywords", "ocr"],
  transfer: { imageMaxEdge: 1024, videoFrames: 5, includeMetadata: true },
  localStt: { enabled: true, model: "base" },
  semanticSearch: { enabled: false },
}

// ─── API ────────────────────────────────────────────────────────────────────

export interface AiApi {
  getAiSettings(): Promise<AiSettings> // get_ai_settings
  updateAiSettings(patch: AiSettingsPatch): Promise<AiSettings> // update_ai_settings { patch }
  giveAiConsent(): Promise<AiSettings> // give_ai_consent
  /** 동의 철회 — 대기 중인 외부 분석 작업을 취소한다. 이미 받은 결과는 보존 */
  revokeAiConsent(): Promise<AiSettings> // revoke_ai_consent

  listAiProviders(): Promise<ProviderStatus[]> // list_ai_providers
  /** 키 쓰기 전용. persist=false 면 이번 실행 동안만 메모리에 둔다 */
  setApiKey(provider: AiProvider, key: string, persist: boolean): Promise<ProviderStatus> // set_api_key { provider, key, persist }
  clearApiKey(provider: AiProvider): Promise<ProviderStatus> // clear_api_key { provider }
  setProviderModel(provider: AiProvider, model: string | null): Promise<ProviderStatus> // set_provider_model { provider, model }
  checkProvider(provider: AiProvider): Promise<ProviderCheck> // check_provider { provider } — 최소 요청 1회
  /** claude 외 subscription 은 invalid_input */
  setProviderAuthMode(provider: AiProvider, mode: AuthMode): Promise<ProviderStatus> // set_provider_auth_mode { provider, mode }
  /** CLI 설치·버전·로그인 상태 재확인 (네트워크 없음, 동의 전 허용) */
  detectAiCli(provider: AiProvider): Promise<ProviderStatus> // detect_ai_cli { provider }
  /** 로그인 명령 실행 → URL 캡처 → Rust 가 기본 브라우저로 연다 */
  aiCliLoginStart(provider: AiProvider): Promise<AiCliLoginState> // ai_cli_login_start { provider }
  aiCliLoginStatus(provider: AiProvider): Promise<AiCliLoginState> // ai_cli_login_status { provider } — done/failed 는 1회만 전달
  aiCliLoginCancel(provider: AiProvider): Promise<AiCliLoginState> // ai_cli_login_cancel { provider }
  /** 캡처한 URL 만 다시 연다 (URL 인자 없음) */
  openAiCliLoginUrl(provider: AiProvider): Promise<boolean> // open_ai_cli_login_url { provider }
  /** 진행 중 외부 AI 요청이 있으면 ai_busy */
  aiCliLogout(provider: AiProvider): Promise<ProviderStatus> // ai_cli_logout { provider }

  listLocalModels(): Promise<LocalModel[]> // list_local_models
  downloadModel(modelId: string): Promise<void> // download_model { modelId }
  onModelsProgress(handler: (p: ModelProgress) => void): Promise<() => void> // event models://progress

  /** 수동·재분석. force=true 면 기존 결과를 덮어쓴다 */
  analyzeAssets(assetIds: number[], kinds: AnalysisKind[] | null, force: boolean): Promise<QueueResult> // analyze_assets { assetIds, kinds, force }
  /** 전체 일괄 (onlyUnanalyzed=false 면 전체 재분석) */
  analyzeAll(onlyUnanalyzed: boolean): Promise<QueueResult> // analyze_all { onlyUnanalyzed }
  /** 자막 없는 동영상·음성 전체에 자막 생성 등록 — 자막 스위치가 꺼져 있으면 invalid_input */
  generateSubtitlesAll(): Promise<QueueResult> // generate_subtitles_all
  retryFailedAnalysis(assetIds: number[] | null): Promise<QueueResult> // retry_failed_analysis { assetIds }
  getAssetAi(assetId: number): Promise<AssetAiDetail> // get_asset_ai { assetId }
  updateAnalysis(assetId: number, edit: AnalysisEdit): Promise<AiAnalysis> // update_analysis { assetId, edit }
  listAnalysisHistory(assetId: number): Promise<AnalysisRun[]> // list_analysis_history { assetId }
  listAiUsage(fromDay: string, toDay: string): Promise<AiUsageRow[]> // list_ai_usage { fromDay, toDay }

  findSimilar(req: SimilarRequest): Promise<Page<SimilarItem>> // find_similar { req }
}

/** `search_assets` 응답에 AI 정보가 붙은 형태 */
export type AiSearchPage = SearchPage & SearchPageAiInfo

// ─── 분석 상태 이벤트 (폴링 대체) ───────────────────────────────────────────

/**
 * `ai://analysis` — 분석 상태가 바뀐 자산의 전체 `AnalysisStatus[]` (자산별 최신 상태, 최대 약 4Hz 로 모아 발행).
 *  - 한 이벤트에 최대 100자산. 바뀌지 않은 자산은 보내지 않는다
 *  - 작업 시작·종료와 사용자의 분석 요청(`analyze_assets`·`retry_failed_analysis`·편집)에서 발행한다.
 *    `analyze_all`·백그라운드/자동 등록처럼 대량으로 등록되는 자산은 이벤트가 없다 → UI 는 폴링(`refetchInterval`)을 유지한다
 *  - 동의 철회로 대기 단계가 건너뜀 처리될 때도 자산별 이벤트는 없다 (철회 command 응답 뒤 재조회)
 */
export const ANALYSIS_EVENT = "ai://analysis"

export interface AnalysisChange {
  assetId: number
  status: AnalysisStatus[]
}

export interface AnalysisEvent {
  changes: AnalysisChange[]
}

/** 새 파일 자동 자막 생성의 실효값 — 값이 있으면 그 값, 없으면(null) 기존 동작대로 "자동 분석 켜짐 + 외부 AI 동의" 를 따른다 */
export function effectiveAutoSubtitle(s: Pick<AiSettings, "autoSubtitleNew" | "autoAnalyzeNew" | "consent">): boolean {
  return s.autoSubtitleNew ?? (s.autoAnalyzeNew && s.consent.given)
}
