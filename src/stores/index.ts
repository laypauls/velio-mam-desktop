import { useActionsStore } from "./actions"
import { useAiStore } from "./ai"
import { usePlaybackStore } from "./playback"
import { useProtectStore } from "./protect"
import { useSearchStore } from "./search"
import { useSelectionStore } from "./selection"
import { usePlayerVolumeStore } from "./playerVolume"
import { useSubtitleStore } from "./subtitle"
import { useUpdateStore } from "./update"
import { useViewerNavStore } from "./viewerNav"
import { useViewStore } from "./view"

export { useActionsStore } from "./actions"
export { useAiStore } from "./ai"
export type { CollectionForm } from "./actions"
export {
  useSearchStore,
  effectiveFilters,
  activeFilterCount,
  isSemanticActive,
  DEFAULT_FILTERS,
  DEFAULT_SORT,
  COLLECTION_SORT,
  SEMANTIC_SORT,
  requestSort,
  canReorder,
  selectCanReorder,
} from "./search"
export type { SidebarView } from "./search"
export { usePlaybackStore } from "./playback"
export type { PlaybackStatus, RepeatMode } from "./playback"
export { useProtectStore } from "./protect"
export type { OriginalsOp, SettingsSection } from "./protect"
export { useSelectionStore } from "./selection"
export { usePlayerVolumeStore } from "./playerVolume"
export { useSubtitleStore } from "./subtitle"
export { useViewStore } from "./view"
export { useViewerNavStore } from "./viewerNav"
export type { GalleryStepper } from "./viewerNav"

/** 테스트용 — 모든 UI 스토어를 초기 상태로 되돌린다. */
export function resetAllStores(): void {
  useSearchStore.setState(useSearchStore.getInitialState(), true)
  useSelectionStore.setState(useSelectionStore.getInitialState(), true)
  useViewStore.setState(useViewStore.getInitialState(), true)
  useActionsStore.setState(useActionsStore.getInitialState(), true)
  useProtectStore.setState(useProtectStore.getInitialState(), true)
  useUpdateStore.setState(useUpdateStore.getInitialState(), true)
  useAiStore.setState(useAiStore.getInitialState(), true)
  usePlaybackStore.setState(usePlaybackStore.getInitialState(), true)
  useSubtitleStore.setState(useSubtitleStore.getInitialState(), true)
  usePlayerVolumeStore.setState(usePlayerVolumeStore.getInitialState(), true)
  useViewerNavStore.setState(useViewerNavStore.getInitialState(), true)
}
