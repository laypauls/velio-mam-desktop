import { useTranslation } from "react-i18next"
import symbol from "@/assets/velio-symbol.svg"

/** 심볼 + 워드마크(`app.title`) + 부제·버전 카드. 워드마크는 텍스트라 테마를 따른다. */
export function BrandMark({ version }: { version: string }) {
  const { t } = useTranslation()
  return (
    <div data-testid="brand-mark" className="flex items-center gap-3 rounded-xl border border-border bg-card px-4 py-3">
      <img src={symbol} alt="" aria-hidden="true" className="size-10 shrink-0" />
      <div className="flex min-w-0 flex-col">
        <span className="text-xl leading-tight font-semibold text-foreground">{t("app.title")}</span>
        <span className="text-xs break-keep text-muted-foreground">
          {t("app.subtitle")} · {version}
        </span>
      </div>
    </div>
  )
}
