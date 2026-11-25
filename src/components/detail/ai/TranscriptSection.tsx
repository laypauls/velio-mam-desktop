import { Play } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { languageText } from "@/components/ai/labels"
import { Button } from "@/components/ui/button"
import { formatDuration } from "@/lib/format"
import type { Transcript } from "@/lib/ipc-contract-ai"
import { useViewStore } from "@/stores"

const PAGE = 50
/** 처음에는 앞 3구간만 보여 주고, "전체 보기" 를 누르면 50구간 단위로 펼친다 */
const PREVIEW = 3

/** D4: STT 원문(감지 언어 표시)과 구간 목록. 구간을 누르면 큰 보기 플레이어가 그 시각으로 이동한다. */
export function TranscriptSection({ assetId, transcript }: { assetId: number; transcript: Transcript | null }) {
  const { t } = useTranslation()
  const seek = useViewStore((s) => s.seekPreview)
  const [shown, setShown] = useState(PREVIEW)

  if (!transcript) {
    return (
      <section className="flex flex-col gap-1">
        <h3 className="text-xs font-medium text-muted-foreground">{t("ai.stt.title")}</h3>
        <p className="text-sm text-muted-foreground">{t("ai.stt.none")}</p>
      </section>
    )
  }
  const segments = transcript.segments.slice(0, shown)
  return (
    <section className="flex flex-col gap-1">
      <h3 className="text-xs font-medium text-muted-foreground">
        {t("ai.stt.titleLang", { lang: languageText(t, transcript.language) })}
      </h3>
      <ul aria-label={t("ai.stt.segments")} className="flex flex-col">
        {segments.map((s) => (
          <li key={s.startMs}>
            <button
              type="button"
              aria-label={t("ai.stt.seek", {
                time: formatDuration(s.startMs),
                text: s.text,
              })}
              onClick={() => seek(assetId, s.startMs)}
              className="flex w-full items-start gap-2 rounded-md px-1 py-1 text-left text-sm hover:bg-muted"
            >
              <span className="shrink-0 text-xs text-muted-foreground tabular-nums">{formatDuration(s.startMs)}</span>
              <span className="min-w-0 flex-1 break-words select-text">{s.text}</span>
              <Play aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
            </button>
          </li>
        ))}
      </ul>
      {shown < transcript.segments.length && (
        <Button
          size="xs"
          variant="ghost"
          className="self-start"
          onClick={() => setShown((n) => (n === PREVIEW ? PAGE : n + PAGE))}
        >
          {shown === PREVIEW ? t("ai.stt.showAll", { count: transcript.segments.length }) : t("ai.stt.more")}
        </Button>
      )}
    </section>
  )
}
