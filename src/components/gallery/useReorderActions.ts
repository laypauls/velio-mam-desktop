import { createContext, useCallback, useContext } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useReorderCollection } from "@/hooks/useCollectionOrder"
import { useToastError } from "@/hooks/useUserData"
import { useApi } from "@/lib/api"
import type { AssetSummary, SearchRequest } from "@/lib/ipc-contract"
import { selectCanReorder, useSearchStore, useSelectionStore } from "@/stores"
import { planReorder, type ReorderOp } from "./reorderTarget"

export interface ReorderActions {
  /** 컬렉션 보기 + 컬렉션 순서 오름차순 — 아니면 메뉴·단축키·드래그가 모두 비활성 */
  enabled: boolean
  /** 컬렉션 보기 안이다 (정렬과 무관) */
  inCollection: boolean
  /** 선택(또는 `ids`)을 한 덩어리로 옮긴다 */
  run: (op: ReorderOp, ids?: readonly number[]) => Promise<void>
  /** `ids` 를 `targetId` 앞으로, null 이면 맨 뒤로 */
  dropBefore: (ids: readonly number[], targetId: number | null) => void
  /** 순서를 바꿀 수 없는 정렬에서 시도했을 때의 안내 */
  hint: () => void
}

const NONE: ReorderActions = { enabled: false, inCollection: false, run: async () => undefined, dropBefore: () => undefined, hint: () => undefined }

/** 갤러리가 제공하는 순서 이동 동작 — 우클릭 메뉴·선택 바가 공유한다 (갤러리 밖에서는 비활성). */
export const ReorderActionsContext = createContext<ReorderActions>(NONE)
export function useReorder(): ReorderActions {
  return useContext(ReorderActionsContext)
}

interface Args {
  getItem: (position: number) => AssetSummary | undefined
  positionOf: (assetId: number) => number | null
  total: number
  /** 현재 갤러리 요청 — 메모리에 없는 이웃 항목을 `at` 으로 조회할 때 쓴다 */
  request: Omit<SearchRequest, "limit" | "cursor">
  /** 메뉴·단축키 이동이 성공하면 이동한 덩어리가 보이도록 갤러리를 그 위치로 따라가게 한다 (드래그는 호출하지 않는다) */
  follow?: (position: number) => void
}

/** 순서 이동 동작 묶음 — 메뉴·단축키·드래그가 같은 경로(`reorder_collection`)를 쓴다. */
export function useReorderActions({ getItem, positionOf, total, request, follow }: Args): ReorderActions {
  const { t } = useTranslation()
  const api = useApi()
  const view = useSearchStore((s) => s.view)
  const enabled = useSearchStore(selectCanReorder)
  const { mutateAsync } = useReorderCollection()
  const toastError = useToastError()
  const collectionId = view.kind === "collection" ? view.collectionId : null

  const idAt = useCallback(
    async (position: number): Promise<number | undefined> => {
      const loaded = getItem(position)
      if (loaded) return loaded.assetId
      if (position < 0 || position >= total) return undefined
      const page = await api.searchAssets({ ...request, cursor: null, at: position, limit: 1 })
      return page.items[0]?.assetId
    },
    [api, getItem, request, total]
  )

  const run = useCallback(
    async (op: ReorderOp, ids?: readonly number[]) => {
      if (!enabled || collectionId === null) return
      const selected = new Set(ids ?? useSelectionStore.getState().ids)
      if (selected.size === 0) return
      try {
        const plan = await planReorder(op, { selected, positionOf, idAt, total })
        if (plan) {
          await mutateAsync({ collectionId, assetIds: [...selected], target: plan.target })
          follow?.(plan.reveal)
        }
      } catch (e) {
        toastError(e)
      }
    },
    [enabled, collectionId, positionOf, idAt, total, mutateAsync, toastError, follow]
  )

  const dropBefore = useCallback(
    (ids: readonly number[], targetId: number | null) => {
      if (!enabled || collectionId === null || ids.length === 0) return
      mutateAsync({ collectionId, assetIds: [...ids], target: targetId === null ? { kind: "end" } : { kind: "before", assetId: targetId } }).catch(toastError)
    },
    [enabled, collectionId, mutateAsync, toastError]
  )

  const hint = useCallback(() => void toast.info(t("order.onlyInCollectionOrder")), [t])

  return { enabled, inCollection: collectionId !== null, run, dropBefore, hint }
}
