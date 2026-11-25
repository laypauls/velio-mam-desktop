import { create } from "zustand"
import type { CollectionRef } from "@/lib/ipc-contract"

/** 컬렉션 이름 Dialog — 새로 만들기(선택적으로 자산을 함께 추가) 또는 이름 변경. */
export type CollectionForm =
  | { mode: "create"; assetIds: number[] }
  | { mode: "rename"; collection: CollectionRef }

interface ActionsState {
  /** 내보내기 Dialog 대상 자산 */
  exportIds: number[] | null
  /** 일괄 태그 Dialog 대상 자산 */
  bulkTagIds: number[] | null
  collectionForm: CollectionForm | null
  deleteCollection: CollectionRef | null
  tagManagerOpen: boolean
  openExport: (ids: number[]) => void
  closeExport: () => void
  openBulkTags: (ids: number[]) => void
  closeBulkTags: () => void
  openCollectionForm: (form: CollectionForm) => void
  closeCollectionForm: () => void
  askDeleteCollection: (c: CollectionRef | null) => void
  setTagManagerOpen: (open: boolean) => void
}

/** 여러 화면(일괄 바·상세 패널·컨텍스트 메뉴·사이드바)이 여는 작업 Dialog 의 열림 상태. */
export const useActionsStore = create<ActionsState>()((set) => ({
  exportIds: null,
  bulkTagIds: null,
  collectionForm: null,
  deleteCollection: null,
  tagManagerOpen: false,
  openExport: (exportIds) => set({ exportIds }),
  closeExport: () => set({ exportIds: null }),
  openBulkTags: (bulkTagIds) => set({ bulkTagIds }),
  closeBulkTags: () => set({ bulkTagIds: null }),
  openCollectionForm: (collectionForm) => set({ collectionForm }),
  closeCollectionForm: () => set({ collectionForm: null }),
  askDeleteCollection: (deleteCollection) => set({ deleteCollection }),
  setTagManagerOpen: (tagManagerOpen) => set({ tagManagerOpen }),
}))
