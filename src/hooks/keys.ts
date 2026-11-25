/** TanStack Query 키 팩토리 — 무효화 범위를 한곳에서 관리한다. */
export const qk = {
  search: ["search"] as const,
  /** 상세 캐시 전체 (접두 무효화용) */
  assets: ["asset"] as const,
  asset: (id: number | null) => ["asset", id] as const,
  folders: ["folders"] as const,
  subfolders: (folderId: number, relPath: string | null) => ["subfolders", folderId, relPath] as const,
  jobs: ["jobs"] as const,
  tags: ["tags"] as const,
  collections: ["collections"] as const,
  /** 선택 자산들의 컬렉션 소속 — `collections` 접두 아래라 컬렉션 구성이 바뀌면 함께 무효화된다 */
  membership: (sortedIds: readonly number[]) => ["collections", "membership", sortedIds] as const,
  /** 연속 재생 대기열 */
  playQueue: (collectionId: number) => ["collections", "playQueue", collectionId] as const,
  /** 자산의 재생 구간 */
  playRanges: (assetId: number) => ["playRanges", assetId] as const,
  /** 컬렉션 대기열의 구간 요약 (접두 무효화용 전체 키) */
  playRangeSummaryAll: ["playRangeSummary"] as const,
  playRangeSummary: (collectionId: number) => ["playRangeSummary", collectionId] as const,
  /** 자산의 장면전환점 (접두 무효화용 전체 키는 `sceneCutsAll`) */
  sceneCuts: (assetId: number) => ["sceneCuts", assetId] as const,
  sceneCutsAll: ["sceneCuts"] as const,
  /** 설정 "모두 추출" 대상 영상 수 */
  sceneBackfillCount: ["sceneBackfillCount"] as const,
  /** 영상 옆 자막 트랙 — 열 때마다 다시 탐색한다 */
  subtitleTracks: (assetId: number) => ["subtitleTracks", assetId] as const,
  subtitleCues: (assetId: number, trackId: string) => ["subtitleCues", assetId, trackId] as const,
  ratingCounts: ["ratingCounts"] as const,
  /** 사용자 정의 메타데이터 필드 정의 (선택지 포함) — 필드가 바뀌면 이 키를 무효화해 구독 화면을 갱신한다 */
  metadataFields: ["metadataFields"] as const,
  /** 필드 삭제 확인 — 값을 가진 자산 수 (열 때마다 새로 조회) */
  metadataFieldValueCount: (fieldId: number) => ["metadataFieldValueCount", fieldId] as const,
  /** 선택지 삭제 확인 — 값으로 가진 자산 수 (열 때마다 새로 조회) */
  metadataOptionUsage: (optionId: number) => ["metadataOptionUsage", optionId] as const,
  recentSearches: ["recentSearches"] as const,
  settings: ["settings"] as const,
  protection: ["protection"] as const,
  scanErrors: ["scanErrors"] as const,
  backups: ["backups"] as const,
  appPaths: ["appPaths"] as const,
  /** 언어 폴더의 언어 파일 목록 — 파일은 앱 밖에서 바뀌므로 새로고침 버튼으로만 다시 받는다 */
  locales: ["locales"] as const,
  startupNotice: ["startupNotice"] as const,
  /** 설정 "기억하는 디스크" */
  knownVolumes: ["knownVolumes"] as const,
  /** 폴더 제거 확인 — 지워질 연결 데이터 개수 (열 때마다 새로 조회) */
  folderRemovalImpact: (folderId: number) => ["folderRemovalImpact", folderId] as const,
  /** 내장 플레이어(워커·libmpv) 사용 가능 여부 — 앱 실행 중 바뀌지 않는다 */
  playerAvailable: ["playerAvailable"] as const,
  /** 이어보기 — 관리 폴더별·전체 기록 수 (설정 페이지) */
  resumeStats: ["resume", "stats"] as const,
  /** 이어보기 — 선택 자산 중 기록이 있는 수 (메뉴 활성화 판정) */
  resumeCount: (sortedIds: readonly number[]) => ["resume", "count", sortedIds] as const,
  /** 이어보기 전체 (접두 무효화용) */
  resumeAll: ["resume"] as const,
  // P3 AI — 키 값은 어떤 캐시에도 넣지 않는다 (공급자 상태는 보관 방식만)
  aiSettings: ["aiSettings"] as const,
  aiProviders: ["aiProviders"] as const,
  aiModels: ["aiModels"] as const,
  aiUsage: (days: number) => ["aiUsage", days] as const,
  assetAi: (id: number | null) => ["assetAi", id] as const,
  assetAiAll: ["assetAi"] as const,
  analysisHistory: (id: number) => ["analysisHistory", id] as const,
  similar: (id: number) => ["similar", id] as const,
  /** 모든 자산의 유사 미디어 조회(설정 변경 시 일괄 무효화) */
  similarAll: ["similar"] as const,
  /** 설치된 플러그인 목록 (설정 > 플러그인) */
  plugins: ["plugins"] as const,
  /** 플러그인별 대기·실패 작업 수 — 작업 요약이 바뀌면 키 접미가 달라져 다시 받는다 */
  pluginCounts: ["pluginCounts"] as const,
  /** 자산 메뉴의 플러그인 기여 항목 — `plugin://changed` 로 무효화 */
  pluginContributions: ["pluginContributions"] as const,
}
