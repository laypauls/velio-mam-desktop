import type { TFunction } from "i18next"
import { Captions } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import type { SubtitleTrack } from "@/lib/ipc-contract"
import type { SubtitleUi } from "./useSubtitles"

/** Radix RadioGroup 값은 빈 문자열을 쓸 수 없다 — "자막 끄기" 는 sentinel 로 렌더하고 `null` 로 되돌린다 */
const OFF = "__off__"
const KNOWN_LANGUAGES = new Set(["ko", "en", "ja", "zh", "fr", "de", "es", "it", "pt", "ru", "vi", "th"])

/** 메뉴 항목 문구 — `한국어 (SMI)`, 같은 언어·형식이 겹치면 파일명을 덧붙인다 */
function trackLabels(tracks: readonly SubtitleTrack[], t: TFunction): string[] {
  const keyOf = (x: SubtitleTrack) => `${x.language ?? ""}|${x.format}`
  const counts = new Map<string, number>()
  for (const x of tracks) counts.set(keyOf(x), (counts.get(keyOf(x)) ?? 0) + 1)
  const seen = new Map<string, number>()
  return tracks.map((x) => {
    const language = x.language === null ? t("viewer.subtitle.unknown") : KNOWN_LANGUAGES.has(x.language) ? t(`viewer.subtitle.lang.${x.language}`) : x.language.toUpperCase()
    const format = x.format.toUpperCase()
    const label = (counts.get(keyOf(x)) ?? 0) > 1 ? t("viewer.subtitle.formatFile", { language, format, file: x.fileName }) : t("viewer.subtitle.format", { language, format })
    // 파일명까지 붙여도 같은 문구(한 파일의 같은 언어 SAMI 클래스 등)면 순번으로 구분한다 — 트랙 id 는 불투명하므로 해석하지 않는다
    const n = (seen.get(label) ?? 0) + 1
    seen.set(label, n)
    return n > 1 ? `${label} (${n})` : label
  })
}

/** 컨트롤 바의 자막 버튼(CC) + 켜기/끄기·언어 선택 라디오 메뉴. 트랙이 없거나 찾는 중이면 비활성 + "자막 없음" 툴팁. */
export function SubtitleMenu({ ui }: { ui: SubtitleUi }) {
  const { t } = useTranslation()
  const label = t("viewer.subtitle.button")
  const pressed = ui.effectiveId !== null
  if (ui.tracks.length === 0) {
    // 비활성 버튼은 포인터 이벤트를 받지 못해 툴팁이 뜨지 않는다 — span 으로 감싼다
    return (
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="inline-flex">
              <Button size="icon-sm" variant="ghost" aria-label={label} aria-pressed={false} disabled>
                <Captions aria-hidden="true" />
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent className="break-keep">{t("viewer.subtitle.none")}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    )
  }
  const labels = trackLabels(ui.tracks, t)
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="icon-sm" variant={pressed ? "secondary" : "ghost"} aria-label={label} aria-pressed={pressed}>
          <Captions aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-w-80 min-w-44">
        <DropdownMenuRadioGroup value={ui.effectiveId ?? OFF} onValueChange={(v) => ui.select(v === OFF ? null : v)}>
          <DropdownMenuRadioItem value={OFF}>{t("viewer.subtitle.off")}</DropdownMenuRadioItem>
          <DropdownMenuSeparator />
          {ui.tracks.map((track, i) => (
            <DropdownMenuRadioItem key={track.id} value={track.id} title={track.fileName} className="break-keep wrap-anywhere">
              {labels[i]}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
