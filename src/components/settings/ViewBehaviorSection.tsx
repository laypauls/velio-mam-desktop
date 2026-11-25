import { useTranslation } from "react-i18next"
import { Checkbox } from "@/components/ui/checkbox"
import { Separator } from "@/components/ui/separator"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { useUpdateSettings } from "@/hooks/useSettings"
import { cn } from "@/lib/utils"
import type { CapturedAtFallback, DoubleClickAction, Settings, SortFieldsEnabled } from "@/lib/ipc-contract"
import { BrowseSettings } from "./BrowseSettings"
import { SettingRow } from "./SettingRow"
import { SettingsSubsection, SubsectionHeading } from "./SettingsSubsection"

/** 끌 수 있는 정렬 항목 — 관련도는 항상 사용(목록에서는 고정 안내만) */
export const TOGGLEABLE_SORT_FIELDS = ["capturedAt", "modifiedAt", "name", "size", "duration", "rating"] as const satisfies readonly (keyof SortFieldsEnabled)[]

const DOUBLE_CLICK_ACTIONS: DoubleClickAction[] = ["preview", "openSystem"]

/** Radix Select 는 빈 문자열 값을 쓸 수 없어 "빈값" 옵션도 "none" 값으로 렌더한다 */
const CAPTURED_AT_FALLBACKS: CapturedAtFallback[] = ["modified", "none"]

/** SelectItem 과 같은 높이·여백의 행 — 행 전체가 label 이라 글자를 눌러도 토글된다. 접근 이름은 항목 라벨만 쓴다(`note` 제외). */
function SortFieldRow({
  label,
  note,
  checked,
  disabled,
  onCheckedChange,
}: {
  label: string
  note?: string
  checked: boolean
  disabled?: boolean
  onCheckedChange?: (on: boolean) => void
}) {
  return (
    <label
      className={cn(
        "flex w-full items-center gap-2 rounded-sm py-1.5 pr-2 pl-2 text-sm select-none hover:bg-accent hover:text-accent-foreground has-focus-visible:bg-accent",
        disabled ? "cursor-not-allowed" : "cursor-pointer",
        !checked && "text-muted-foreground"
      )}
    >
      <Checkbox
        aria-label={label}
        checked={checked}
        disabled={disabled}
        onCheckedChange={(v) => onCheckedChange?.(v === true)}
      />
      <span className="flex-1 break-keep">{label}</span>
      {note && <span className="text-xs text-muted-foreground">{note}</span>}
    </label>
  )
}

/** 설정 > "탐색·표시" 페이지 본문(제목은 페이지 헤더): 목록 탐색 / 정렬 항목 / 콘텐츠(제목 편집·더블클릭·촬영일) / 사이드바. */
export function ViewBehaviorSection({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  const enabled = settings.sortFieldsEnabled
  const enabledCount = TOGGLEABLE_SORT_FIELDS.filter((f) => enabled[f]).length

  return (
    <section aria-label={t("settings.view.section")} className="flex flex-col gap-4">
      <BrowseSettings settings={settings} />
      <Separator />
      <div role="group" aria-label={t("settings.view.sortFields")} className="flex flex-col gap-3">
        <SubsectionHeading title={t("settings.view.sortFields")} description={t("settings.view.sortFieldsHint")} />
        {/* 툴바 정렬 드롭다운(SelectContent)과 같은 모양의 정적 패널 */}
        <div className="w-56 rounded-md bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10">
          <SortFieldRow label={t("sort.field.relevance")} note={t("settings.view.alwaysShort")} checked disabled />
          {TOGGLEABLE_SORT_FIELDS.map((f) => (
            <SortFieldRow
              key={f}
              label={t(`sort.field.${f}`)}
              note={enabled[f] ? undefined : t("settings.view.sortFieldHidden")}
              checked={enabled[f]}
              // 마지막 하나는 끌 수 없다 — 정렬 드롭다운이 관련도만 남지 않게
              disabled={enabled[f] && enabledCount <= 1}
              onCheckedChange={(on) => update.mutate({ sortFieldsEnabled: { ...enabled, [f]: on } })}
            />
          ))}
        </div>
      </div>
      <Separator />
      <SettingsSubsection title={t("settings.sub.content")}>
        <SettingRow label={t("settings.view.titleEdit")} description={t("settings.view.titleEditHint")}>
          <Switch
            aria-label={t("settings.view.titleEdit")}
            checked={settings.titleEditEnabled}
            onCheckedChange={(titleEditEnabled) => update.mutate({ titleEditEnabled })}
          />
        </SettingRow>
        <SettingRow label={t("settings.view.doubleClick")} description={t("settings.view.doubleClickHint")}>
          <Select
            value={settings.doubleClickAction}
            onValueChange={(v) => update.mutate({ doubleClickAction: v as DoubleClickAction })}
          >
            <SelectTrigger aria-label={t("settings.view.doubleClick")} className="w-52">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {DOUBLE_CLICK_ACTIONS.map((a) => (
                <SelectItem key={a} value={a}>
                  {t(a === "preview" ? "settings.view.doubleClickPreview" : "settings.view.doubleClickOpenSystem")}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingRow>
        <SettingRow label={t("settings.view.capturedFallback")} description={t("settings.view.capturedFallbackHint")}>
          <Select
            value={settings.capturedAtFallback}
            onValueChange={(v) => update.mutate({ capturedAtFallback: v as CapturedAtFallback })}
          >
            <SelectTrigger aria-label={t("settings.view.capturedFallback")} className="w-52">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {CAPTURED_AT_FALLBACKS.map((m) => (
                <SelectItem key={m} value={m}>
                  {t(m === "modified" ? "settings.view.capturedFallbackModified" : "settings.view.capturedFallbackNone")}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingRow>
      </SettingsSubsection>
      <Separator />
      <SettingsSubsection title={t("settings.sub.sidebar")}>
        <SettingRow label={t("settings.view.sidebarRating")} description={t("settings.view.sidebarRatingHint")}>
          <Switch
            aria-label={t("settings.view.sidebarRating")}
            checked={settings.sidebarRatingMenu}
            onCheckedChange={(sidebarRatingMenu) => update.mutate({ sidebarRatingMenu })}
          />
        </SettingRow>
      </SettingsSubsection>
    </section>
  )
}
