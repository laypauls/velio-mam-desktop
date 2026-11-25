import { useTranslation } from "react-i18next"
import { RatingInput } from "@/components/rating/RatingInput"
import { useSetRating, useToastError } from "@/hooks/useUserData"

/** 상세 패널 별점 입력 — 같은 별을 다시 누르면 해제. */
export function AssetRating({ assetId, rating }: { assetId: number; rating: number }) {
  const { t } = useTranslation()
  const setRating = useSetRating()
  const toastError = useToastError()
  return (
    <div className="flex items-center gap-2 text-sm">
      <span className="text-muted-foreground">{t("rating.label")}</span>
      <RatingInput value={rating} disabled={setRating.isPending} onChange={(v) => setRating.mutate({ assetIds: [assetId], rating: v }, { onError: toastError })} />
    </div>
  )
}
