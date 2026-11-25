import type { FolderRemovalImpact, IgnoredVolume, KnownVolume, PendingVolume } from "@/lib/ipc-contract-volume"
import type { BackupInfo, Settings, StartupNotice, UpdateError } from "@/lib/ipc-contract"
import type { AiProvider, AnalysisKind } from "@/lib/ipc-contract-ai"
import type { SubtitleCue, SubtitleTrack } from "@/lib/ipc-contract"
import type { MockDb, Rec } from "./mock-data"

export interface MockOptions {
  /** 합성 자산 수 (기본 100,000) */
  count?: number
  seed?: number
  /** 현재 시각 주입 (인증 면제 시간 계산) */
  now?: () => number
  /** 모든 command 에 추가하는 지연(ms) */
  delayMs?: number
  /** 비밀번호를 미리 설정 */
  password?: string
  /** 목록에 미리 넣어 둘 백업 (최신 순) */
  backups?: BackupInfo[]
  /** `takeStartupNotice` 가 1회 돌려줄 기동 알림 */
  startupNotice?: StartupNotice
  /** 최초 실행 안내 대기 상태로 시작 */
  firstRun?: boolean
  /** 스캔 진행 중 상태로 시작 (정적 42%) */
  scanning?: boolean
  /** 설치 가능한 업데이트를 돌려준다 (기본: 업데이트 없음) */
  update?: { version: string; notes?: string; date?: string; currentVersion?: string }
  /** `checkUpdate` 가 이 코드로 reject */
  updateCheckError?: UpdateError["code"]
  /** `installUpdate` 가 이 코드로 reject */
  updateInstallError?: UpdateError["code"]
  /** P3 AI mock 동작 */
  ai?: MockAiOptions
  /** 자산별 자막 (트랙 + 큐) — 기본: 자막 없음 */
  subtitles?: Record<number, { track: SubtitleTrack; cues: SubtitleCue[] }[]>
  /** 장면 검출 mock — 요청한 영상이 `stepMs` 뒤 done 으로 바뀌고 `scene://updated` 가 온다 (타이머 주입 가능) */
  scene?: { schedule?: (fn: () => void, ms: number) => void; stepMs?: number }
  /** 새 디스크 mock 시드 — pending: 대기 목록, ignored: 묻지 않는 디스크, connected: 다시 묻기 때 대기로 올라올 연결 중 디스크 */
  volumes?: {
    pending?: PendingVolume[]
    ignored?: IgnoredVolume[]
    connected?: PendingVolume[]
    /** 기억하는 디스크 — volumes 행이 있는 디스크 (묻지 않음 전용 항목은 `ignored` 에서 병합) */
    known?: KnownVolume[]
    /** `folderRemovalImpact` 가 돌려줄 개수 (기본: 폴더의 활성 자산 수, 나머지 0) */
    removalImpact?: FolderRemovalImpact
  }
  /** 테스트 전용: `value` 를 올리면 그 전에 발급된 hybrid 커서가 `invalid_input` 으로 거부된다 */
  searchGeneration?: { value: number }
}

/** P3 AI mock 옵션. 타이머는 주입할 수 있어 테스트에서 결정적으로 진행시킨다. */
export interface MockAiOptions {
  /** 외부 전송 동의를 이미 한 상태로 시작 */
  consent?: boolean
  /** multilingual-e5 텍스트 모델이 준비된 상태로 시작 (의미 검색 degraded 해제) */
  e5Ready?: boolean
  /** CLIP 모델이 준비된 상태로 시작 (유사 미디어·시각 임베딩) */
  clipReady?: boolean
  /** `checkProvider` 결과 오류 코드 (공급자별, 기본 ok) */
  checkErrors?: Partial<Record<AiProvider, string>>
  /** 구독(Claude Code CLI) mock — 기본: 설치됨·지원 버전·로그아웃 상태 */
  cli?: {
    found?: boolean
    supported?: boolean
    loggedIn?: boolean
    /** 상태 명령 실패 → 계정 알 수 없음(null) */
    accountUnknown?: boolean
    apiKeyEnv?: boolean
    /** aiCliLogout 이 ai_busy 로 거부 */
    busy?: boolean
    /** 로그인이 완료로 바뀌기까지 status 조회 횟수 (기본 2) */
    loginPolls?: number
  }
  /** 종류별 분석 실패 코드 (예: ocr → "offline"). 재시도 때는 적용하지 않는다 */
  failKinds?: Partial<Record<AnalysisKind, string>>
  /** 모델 다운로드를 이 코드로 실패시킨다 (예: "offline") */
  downloadError?: string
  /** 타이머 주입 (기본 setTimeout). 분석 대기→실행→완료, 모델 다운로드 진행에 쓴다 */
  schedule?: (fn: () => void, ms: number) => void
  /** 상태 전이 간격(ms, 기본 700) */
  stepMs?: number
}

/** mock 하위 모듈이 공유하는 컨텍스트. */
export interface MockCtx {
  opts: MockOptions
  count: number
  now: () => number
  settings: Settings
  db: () => MockDb
  rec: (id: number) => Rec
  wait: () => Promise<void>
}
