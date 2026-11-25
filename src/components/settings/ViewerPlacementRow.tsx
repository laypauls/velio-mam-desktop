import { useTranslation } from "react-i18next"
import { CardRadioGroup, type CardOption } from "@/components/settings/CardRadioGroup"
import { PlacementPreview } from "@/components/settings/PlacementPreview"
import { useUpdateSettings } from "@/hooks/useSettings"
import type { ViewerPlacement } from "@/lib/ipc-contract"

/** 설정 > 플레이어 > 재생: 플레이어 패널(콘텐츠 보기 영역)을 창 맨 왼쪽(사이드바 앞, 기본)과 갤러리 오른쪽 중 어디에 둘지 — 카드는 왼쪽 → 오른쪽 순 */
export function ViewerPlacementRow({ value }: { value: ViewerPlacement }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  const options: CardOption<ViewerPlacement>[] = [
    {
      value: "left",
      label: t("settings.player.viewerPlacementLeft"),
      hint: t("settings.player.viewerPlacementLeftHint"),
      preview: <PlacementPreview placement="left" />,
    },
    {
      value: "right",
      label: t("settings.player.viewerPlacementRight"),
      hint: t("settings.player.viewerPlacementRightHint"),
      preview: <PlacementPreview placement="right" />,
    },
  ]
  return (
    <CardRadioGroup
      title={t("settings.player.viewerPlacement")}
      description={t("settings.player.viewerPlacementHint")}
      value={value}
      options={options}
      onChange={(v) => update.mutate({ viewerPlacement: v === "right" ? "right" : "left" })}
    />
  )
}
