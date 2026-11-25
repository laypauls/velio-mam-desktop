import { useTranslation } from "react-i18next"
import { RANGE_KEY_PRESETS, rangeKeyLabel, type RangeKeyActionType } from "@/components/viewer/playRangeKeys"
import { useUpdateSettings } from "@/hooks/useSettings"
import type { PlayRangeKeys } from "@/lib/ipc-contract"
import { CardRadioGroup, type CardOption } from "./CardRadioGroup"
import { KeyCombo } from "./KeyCombo"

const PRESETS: PlayRangeKeys[] = ["brackets", "premiere"]

/** 안내에 보이는 동작 순서 (읽기 전용 키 안내) */
const GUIDE_ORDER: RangeKeyActionType[] = ["markIn", "markOut", "gotoPrev", "gotoNext", "selectAtPlayhead", "clearIn", "clearOut", "deleteAll", "deleteSelected"]

/** 프리셋 카드의 미리보기 — 시작(In)·끝(Out) 키를 짧은 타임라인 양 끝의 키캡으로 보인다 (장식, 전체 키는 아래 표) */
function PresetPreview({ preset }: { preset: PlayRangeKeys }) {
  const markIn = rangeKeyLabel(preset, "markIn")
  const markOut = rangeKeyLabel(preset, "markOut")
  return (
    <div className="flex flex-1 items-center justify-center gap-2 px-2">
      {markIn && <KeyCombo shortcut={markIn} />}
      <div className="h-0.5 w-8 rounded-sm bg-muted-foreground/40" />
      {markOut && <KeyCombo shortcut={markOut} />}
    </div>
  )
}

/**
 * 설정 > 플레이어 > 단축키: 재생 구간 단축키 프리셋(괄호 / Premiere) 카드 + 선택한 프리셋의 키 안내 표(키캡).
 * 숨기기·패널 닫기 단축키와 겹치면(`conflicted`) 라디오 그룹에 aria-invalid 를 표시한다 — 경고 문구는 단축키 소제목이 한 번만 보이고(`warnId` 로 연결),
 * 겹친 키는 단축키가 먼저 처리된다. 옵션 값은 항상 있어 빈 값이 없다.
 */
export function PlayRangeKeysRow({ value, conflicted, warnId }: { value: PlayRangeKeys; conflicted: boolean; warnId: string }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  const options: CardOption<PlayRangeKeys>[] = PRESETS.map((p) => ({ value: p, label: t(`settings.view.rangeKeys_${p}`), preview: <PresetPreview preset={p} /> }))
  return (
    <div className="flex flex-col gap-2">
      <CardRadioGroup
        title={t("settings.view.rangeKeys")}
        description={t("settings.view.rangeKeysHint")}
        value={value}
        options={options}
        onChange={(playRangeKeys) => update.mutate({ playRangeKeys })}
        invalid={conflicted}
        describedBy={conflicted ? warnId : undefined}
        columnsClassName="sm:max-w-md"
      />
      <dl aria-label={t("settings.view.rangeKeysGuide")} className="grid grid-cols-[auto_1fr] items-center gap-x-4 gap-y-1 rounded-lg bg-muted/50 px-3 py-2 text-xs">
        {GUIDE_ORDER.map((action) => {
          const binding = RANGE_KEY_PRESETS[value].find((b) => b.action === action)
          return (
            <div key={action} className="contents">
              <dt className="text-muted-foreground break-keep">{t(`settings.view.rangeAction_${action}`)}</dt>
              <dd className="font-medium">{binding ? <KeyCombo shortcut={binding.label} /> : ""}</dd>
            </div>
          )
        })}
      </dl>
    </div>
  )
}
