import { Sparkles, Type } from "lucide-react"
import { toast } from "sonner"
import { useTranslation } from "react-i18next"
import { Toggle } from "@/components/ui/toggle"
import { useModelReadiness } from "@/hooks/useAi"
import { useProtectStore, useSearchStore } from "@/stores"

/** D3: 키워드 ↔ 의미 검색 단일 토글(눌림 = 의미). 임베딩 모델이 없으면 켤 때 다운로드 안내(설정 > AI)를 띄운다. */
export function SearchModeToggle() {
  const { t } = useTranslation()
  const mode = useSearchStore((s) => s.mode)
  const setMode = useSearchStore((s) => s.setMode)
  const openSettings = useProtectStore((s) => s.openSettings)
  const { loaded, e5Ready } = useModelReadiness()
  const semantic = mode === "hybrid"
  const Icon = semantic ? Sparkles : Type

  const onPressedChange = (pressed: boolean) => {
    setMode(pressed ? "hybrid" : "keyword")
    if (!pressed) return
    if (loaded && !e5Ready) {
      toast.info(t("search.mode.needModels"), {
        description: t("search.mode.needModelsHint"),
        action: { label: t("search.mode.openSettings"), onClick: () => openSettings("ai-local") },
      })
    }
  }

  return (
    <Toggle
      pressed={semantic}
      onPressedChange={onPressedChange}
      aria-label={t("search.mode.semantic")}
      title={t(semantic ? "search.mode.semantic" : "search.mode.keyword")}
      className="h-6 min-w-6 px-1 text-muted-foreground data-[state=on]:bg-accent data-[state=on]:text-accent-foreground data-[state=on]:hover:bg-accent data-[state=on]:hover:text-accent-foreground"
    >
      <Icon aria-hidden="true" className="size-3.5" />
    </Toggle>
  )
}
