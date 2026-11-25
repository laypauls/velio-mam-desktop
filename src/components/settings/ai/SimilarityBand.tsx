import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { BAND_CELLS, cellZone, markerCell, similarityZone, type BandZone } from "./bandZones"

const ZONE_CLASS: Record<BandZone, string> = {
  weak: "bg-destructive/30",
  mid: "bg-muted",
  recommended: "bg-primary/40",
}

/**
 * 최소 유사도 슬라이더 아래의 구간 띠 — 0~60% 는 무관한 이미지가 섞이는 구간, 70%+ 는 권장 구간이고 현재 값에 마커를 표시한다.
 * 띠는 장식(aria-hidden)이고, 현재 값이 어느 구간인지는 한 줄 문장으로도 보여 스크린리더가 읽는다. 0% 는 "제한 없음".
 */
export function SimilarityBand({ percent }: { percent: number }) {
  const { t } = useTranslation()
  const marker = markerCell(percent)
  const status = percent <= 0 ? t("ai.similarSettings.bandNone") : t(`ai.similarSettings.band_${similarityZone(percent)}`, { percent })
  return (
    <div className="flex flex-col gap-1.5">
      <div aria-hidden="true" className="flex flex-col gap-1">
        <div className="grid grid-cols-20 gap-px overflow-hidden rounded-sm">
          {Array.from({ length: BAND_CELLS }, (_, i) => (
            <div key={i} className={cn("relative h-3", ZONE_CLASS[cellZone(i)])}>
              {i === marker && <div className="absolute inset-y-0 left-1/2 w-0.5 -translate-x-1/2 bg-foreground" />}
            </div>
          ))}
        </div>
        <div className="grid grid-cols-10 text-[10px] leading-none text-muted-foreground">
          <span className="col-span-6">{t("ai.similarSettings.bandWeak")}</span>
          <span className="col-span-1" />
          <span className="col-span-3 text-primary">{t("ai.similarSettings.bandRecommended")}</span>
        </div>
      </div>
      <p role="status" className="text-xs text-muted-foreground break-keep">
        {status}
      </p>
    </div>
  )
}
