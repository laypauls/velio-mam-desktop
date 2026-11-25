import { Heart } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useSetFavorite, useToastError } from "@/hooks/useUserData"

/** 제목 줄 즐겨찾기 하트 토글. */
export function FavoriteButton({ assetId, favorite }: { assetId: number; favorite: boolean }) {
  const { t } = useTranslation()
  const setFavorite = useSetFavorite()
  const toastError = useToastError()
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-xs"
      aria-label={t("selection.action.favorite")}
      title={t(favorite ? "favorite.off" : "favorite.on")}
      aria-pressed={favorite}
      disabled={setFavorite.isPending}
      onClick={() => setFavorite.mutate({ assetIds: [assetId], favorite: !favorite }, { onError: toastError })}
    >
      <Heart aria-hidden="true" className={favorite ? "fill-current text-favorite" : undefined} />
    </Button>
  )
}
