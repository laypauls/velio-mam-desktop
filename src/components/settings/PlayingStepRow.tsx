import { useTranslation } from "react-i18next"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { PLAYING_STEP_OPTIONS, normalizePlayingStep } from "@/hooks/usePlayingStepSec"
import { useUpdateSettings } from "@/hooks/useSettings"
import { SettingRow } from "./SettingRow"

/** 설정 > 플레이어: 재생 중 뒤로·앞으로 이동 폭(초). 정지 중에는 항상 1프레임 */
export function PlayingStepRow({ value }: { value: number }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  const current = normalizePlayingStep(value)
  // 저장값이 선택지에 없으면(다른 도구로 바꾼 값 등) 그 값도 보여 준다
  const options = (PLAYING_STEP_OPTIONS as readonly number[]).includes(current) ? PLAYING_STEP_OPTIONS : [...PLAYING_STEP_OPTIONS, current].sort((a, b) => a - b)
  return (
    <SettingRow label={t("settings.player.playingStep")} description={t("settings.player.playingStepHint")}>
      <Select value={String(current)} onValueChange={(v) => update.mutate({ playerPlayingStepSec: Number(v) })}>
        <SelectTrigger aria-label={t("settings.player.playingStep")} className="w-28">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((sec) => (
            <SelectItem key={sec} value={String(sec)}>
              {t("settings.player.playingStepOption", { sec })}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </SettingRow>
  )
}
