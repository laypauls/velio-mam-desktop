import { create } from "zustand"
import type { SearchFilters, SortSpec } from "@/lib/ipc-contract"
import type { SearchMode } from "@/lib/ipc-contract-ai"

/** 사이드바에서 선택한 라이브러리 보기. */
export type SidebarView =
  | { kind: "all" }
  | { kind: "image" }
  | { kind: "video" }
  | { kind: "audio" }
  | { kind: "favorites" }
  | { kind: "rating"; value: 0 | 1 | 2 | 3 | 4 | 5 }
  | { kind: "collection"; collectionId: number }
  | { kind: "folder"; folderId: number; relPath: string | null }

/** PL5 기본: 오프라인 표시, 파일 없음 숨김. */
export const DEFAULT_FILTERS: SearchFilters = {
  mediaTypes: [],
  includeOffline: true,
  includeMissing: false,
  capturedFrom: null,
  capturedTo: null,
  sizeMin: null,
  sizeMax: null,
  durationMinMs: null,
  durationMaxMs: null,
  folder: null,
  favoritesOnly: false,
  ratingMin: null,
  ratingMax: null,
  collectionId: null,
  tagIds: [],
}

export const DEFAULT_SORT: SortSpec = { field: "capturedAt", direction: "desc" }

/** 컬렉션 보기에 들어가면 쓰는 정렬 — 방향은 오름차순으로 고정한다 */
export const COLLECTION_SORT: SortSpec = { field: "collectionOrder", direction: "asc" }

interface SearchState {
  view: SidebarView
  /** 디바운스를 거쳐 확정된 질의 */
  query: string
  /** 키워드 ↔ 의미(hybrid) 검색 모드 — 의미 모드는 질의가 있을 때만 요청에 실린다 */
  mode: SearchMode
  filters: SearchFilters
  sort: SortSpec
  /** 컬렉션 보기에 들어가기 직전의 정렬 — 나갈 때 복원한다 */
  sortBeforeCollection: SortSpec | null
  setView: (view: SidebarView) => void
  setQuery: (query: string) => void
  setMode: (mode: SearchMode) => void
  patchFilters: (patch: Partial<SearchFilters>) => void
  resetFilters: () => void
  setSort: (sort: SortSpec) => void
}

export const useSearchStore = create<SearchState>()((set) => ({
  view: { kind: "all" },
  query: "",
  mode: "keyword",
  filters: DEFAULT_FILTERS,
  sort: DEFAULT_SORT,
  sortBeforeCollection: null,
  setView: (view) =>
    set((s) => {
      if (view.kind === "collection") {
        // 컬렉션끼리 옮길 때는 이전 정렬 기억을 그대로 둔다
        const saved = s.view.kind === "collection" || s.sort.field === "collectionOrder" ? s.sortBeforeCollection : s.sort
        return { view, sort: COLLECTION_SORT, sortBeforeCollection: saved }
      }
      if (s.sort.field === "collectionOrder") return { view, sort: s.sortBeforeCollection ?? DEFAULT_SORT, sortBeforeCollection: null }
      return { view }
    }),
  setQuery: (query) => set({ query }),
  setMode: (mode) => set({ mode }),
  patchFilters: (patch) => set((s) => ({ filters: { ...s.filters, ...patch } })),
  resetFilters: () => set({ filters: DEFAULT_FILTERS }),
  setSort: (sort) => set({ sort }),
}))

/** 의미 모드는 질의가 있을 때만 유효하다 (빈 질의는 목록 조회). */
export function isSemanticActive(mode: SearchMode, query: string): boolean {
  return mode === "hybrid" && query !== ""
}

/** 의미 검색은 관련도순으로만 정렬된다 — 스토어의 사용자 정렬은 건드리지 않고 요청에만 적용한다. */
export const SEMANTIC_SORT: SortSpec = { field: "relevance", direction: "desc" }

/** 사용자 필터에 사이드바 보기를 합쳐 실제 검색 필터를 만든다. */
export function effectiveFilters(view: SidebarView, filters: SearchFilters): SearchFilters {
  switch (view.kind) {
    case "image":
    case "video":
    case "audio":
      return { ...filters, mediaTypes: [view.kind] }
    case "favorites":
      return { ...filters, favoritesOnly: true }
    case "rating":
      return { ...filters, ratingMin: view.value, ratingMax: view.value }
    case "collection":
      return { ...filters, collectionId: view.collectionId }
    case "folder":
      return { ...filters, folder: { folderId: view.folderId, relPath: view.relPath, recursive: true } }
    default:
      return filters
  }
}

/**
 * 요청에 실리는 정렬 — `collectionOrder` 는 유효한 컬렉션이 있을 때만 쓴다(없으면 백엔드가 invalid_input).
 * 컬렉션 보기 밖에서 이 정렬이 남아 있어도 기본 정렬로 돌린다.
 */
export function requestSort(view: SidebarView, filters: SearchFilters, sort: SortSpec): SortSpec {
  if (sort.field !== "collectionOrder") return sort
  return effectiveFilters(view, filters).collectionId === null ? DEFAULT_SORT : sort
}

/** 순서 바꾸기(메뉴·단축키·드래그)가 가능한 상태 — 컬렉션 보기 + 컬렉션 순서 오름차순. 의미 검색은 관련도 정렬이라 자연히 제외된다 */
export function canReorder(view: SidebarView, effectiveSort: SortSpec): boolean {
  return view.kind === "collection" && effectiveSort.field === "collectionOrder" && effectiveSort.direction === "asc"
}

/** zustand selector — 지금 화면에서 순서를 바꿀 수 있는가 */
export function selectCanReorder(s: SearchState): boolean {
  return canReorder(s.view, isSemanticActive(s.mode, s.query) ? SEMANTIC_SORT : requestSort(s.view, s.filters, s.sort))
}

/** 기본값과 다른 사용자 필터 개수 (필터 버튼 배지). */
export function activeFilterCount(f: SearchFilters): number {
  const d = DEFAULT_FILTERS
  const base = [
    f.mediaTypes.length > 0,
    f.includeOffline !== d.includeOffline,
    f.includeMissing !== d.includeMissing,
    f.capturedFrom !== null || f.capturedTo !== null,
    f.sizeMin !== null || f.sizeMax !== null,
    f.durationMinMs !== null || f.durationMaxMs !== null,
    f.favoritesOnly,
    f.ratingMin !== null || f.ratingMax !== null,
  ].filter(Boolean).length
  // 사용자 메타데이터는 필드마다 1개 (필드당 조건은 최대 1개)
  return base + (f.userFields?.length ?? 0)
}
