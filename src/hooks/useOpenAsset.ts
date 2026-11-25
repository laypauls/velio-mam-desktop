import { useCallback } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { unavailableReason } from "@/components/detail/AssetActions"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import type { Availability } from "@/lib/ipc-contract"
import { useViewStore } from "@/stores"
import { useSettings } from "./useSettings"

/**
 * 오프라인·파일 없음 자산은 콘텐츠 보기를 열지 않고 사유만 toast 로 알린다(미디어 종류 무관).
 * 카드 빠른 동작 "크게 보기" 등 미리보기를 직접 여는 진입점이 공유하는 가드.
 */
export function useOpenPreview(): (assetId: number, availability: Availability) => void {
  const { t } = useTranslation()
  return useCallback(
    (assetId, availability) => {
      const reason = unavailableReason(availability, t)
      if (reason !== null) toast.info(reason)
      else useViewStore.getState().openPreview(assetId)
    },
    [t]
  )
}

/**
 * 콘텐츠 열기(더블클릭·Enter 공통). 오프라인·파일 없음이면 설정과 무관하게 사유 toast 만 띄우고 열지 않는다.
 * 설정 `doubleClickAction` 이 "openSystem" 이면 기본 프로그램으로 열고, 호출이 실패하면 미리보기로 대체하며 사유를 toast 로 알린다.
 */
export function useOpenAsset(): (assetId: number, availability: Availability) => void {
  const api = useApi()
  const { t } = useTranslation()
  const openPreview = useOpenPreview()
  const action = useSettings().data?.doubleClickAction ?? "preview"
  return useCallback(
    (assetId, availability) => {
      if (action !== "openSystem" || unavailableReason(availability, t) !== null) return openPreview(assetId, availability)
      api.openOriginal(assetId).catch((e: unknown) => {
        toast.error(t("open.fallbackToPreview", { reason: t(errorKey(e)) }))
        useViewStore.getState().openPreview(assetId)
      })
    },
    [action, api, t, openPreview]
  )
}
