import { Film, Loader2, SkipBack, SkipForward } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { formatNumber } from "@/lib/format"
import type { SceneUi } from "./useSceneController"

/**
 * 컨트롤 바의 장면전환점 그룹 (필름 아이콘).
 * - 추출 전/사용자 컷만 있음: 작은 "장면 추출" 칩 / 대기·실행 중: 비활성 스피너 칩(aria-busy) / 실패: 경고색 "장면 다시 시도"(사유 툴팁)
 * - 컷이 1개 이상: `[이전 장면][장면 N 표시 토글][다음 장면]` — 이전/다음은 `Ctrl+←/→` 와 같은 이동이고 표시 토글이 꺼져 있어도 동작한다. 앞/뒤에 컷이 없으면 비활성
 * 지원하지 않는 영상(스트림 없음 등)은 추출 버튼을 보이지 않는다. 좁은 컨테이너에서는 글자를 접근성 전용으로 줄이고 아이콘+툴팁만 남긴다.
 */
export function SceneControls({ ui }: { ui: SceneUi }) {
  const { t } = useTranslation()
  const count = ui.cuts.length
  const showToggle = ui.state === "done" || count > 0
  const label = (text: string) => <span className="@max-lg:sr-only">{text}</span>
  return (
    <div className="flex items-center gap-1">
      {count > 0 && (
        <Button size="icon-sm" variant="ghost" aria-label={t("scene.prev")} title={t("viewer.withKey", { label: t("scene.prev"), key: "Ctrl+←" })} disabled={!ui.canMovePrev} onClick={() => ui.move(-1)}>
          <SkipBack aria-hidden="true" />
        </Button>
      )}
      {showToggle && (
        <Button
          size="sm"
          variant={ui.visible ? "secondary" : "ghost"}
          aria-pressed={ui.visible}
          aria-label={t("scene.toggleLabel", { count })}
          title={t("scene.toggleHint")}
          onClick={ui.toggleVisible}
        >
          <Film aria-hidden="true" />
          {label(t("scene.toggle", { count: formatNumber(count) }))}
        </Button>
      )}
      {count > 0 && (
        <Button size="icon-sm" variant="ghost" aria-label={t("scene.next")} title={t("viewer.withKey", { label: t("scene.next"), key: "Ctrl+→" })} disabled={!ui.canMoveNext} onClick={() => ui.move(1)}>
          <SkipForward aria-hidden="true" />
        </Button>
      )}
      {ui.state === "pending" && (
        <Button size="xs" variant="ghost" disabled aria-busy="true">
          <Loader2 aria-hidden="true" className="animate-spin" />
          {label(t("scene.extracting"))}
        </Button>
      )}
      {ui.canExtract && (
        <Button size="xs" variant="outline" title={t("scene.extractHint")} onClick={ui.extract}>
          <Film aria-hidden="true" />
          {label(t("scene.extract"))}
        </Button>
      )}
      {ui.state === "failed" && (
        <Button size="xs" variant="destructive" title={ui.error !== null ? t(`scene.failure.${ui.error}`) : t("scene.retry")} onClick={ui.extract}>
          <Film aria-hidden="true" />
          {label(t("scene.retry"))}
        </Button>
      )}
    </div>
  )
}
