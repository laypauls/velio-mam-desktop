import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useUpdateSettings } from "@/hooks/useSettings"
import { errorKey } from "@/lib/errors"
import type { GalleryPaging, IndexRailMode, Settings, SettingsPatch } from "@/lib/ipc-contract"
import { PagingPreview, RailPreview } from "./BrowsePreviews"
import { CardRadioGroup, type CardOption } from "./CardRadioGroup"
import { SettingRow } from "./SettingRow"
import { SubsectionHeading } from "./SettingsSubsection"

const PAGE_SIZES: Settings["galleryPageSize"][] = [50, 100, 200]
const RAIL_MODES: IndexRailMode[] = ["auto", "always", "off"]

const RAIL_KEY = { auto: "settings.view.indexRailAuto", always: "settings.view.indexRailAlways", off: "settings.view.indexRailOff" } as const
const RAIL_HINT_KEY = { auto: "settings.view.indexRailAutoHint", always: "settings.view.indexRailAlwaysHint", off: "settings.view.indexRailOffHint" } as const

/** 설정 > 탐색·표시 > "목록 탐색": 탐색 방식·목차 레일(미리보기 카드), 페이지당 개수(페이지 모드만). 갤러리는 설정 캐시를 읽어 바로 반응한다. */
export function BrowseSettings({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  const [error, setError] = useState<string | null>(null)

  const save = (patch: SettingsPatch) => {
    setError(null)
    update.mutate(patch, { onError: (e) => setError(t(errorKey(e))) })
  }

  const pagingOptions: CardOption<GalleryPaging>[] = [
    { value: "infinite", label: t("settings.view.pagingInfinite"), hint: t("settings.view.pagingInfiniteHint"), preview: <PagingPreview mode="infinite" /> },
    { value: "paged", label: t("settings.view.pagingPaged"), hint: t("settings.view.pagingPagedHint"), preview: <PagingPreview mode="paged" /> },
  ]
  const railOptions: CardOption<IndexRailMode>[] = RAIL_MODES.map((m) => ({ value: m, label: t(RAIL_KEY[m]), hint: t(RAIL_HINT_KEY[m]), preview: <RailPreview mode={m} /> }))

  return (
    <div role="group" aria-label={t("settings.view.browse")} className="flex flex-col gap-3">
      <SubsectionHeading title={t("settings.view.browse")} description={t("settings.view.browseHint")} />
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
      <CardRadioGroup
        title={t("settings.view.paging")}
        value={settings.galleryPaging}
        options={pagingOptions}
        onChange={(galleryPaging) => save({ galleryPaging })}
        columnsClassName="sm:max-w-md"
      />
      <SettingRow label={t("settings.view.pageSize")} description={t("settings.view.pageSizeHint")}>
        <Select
          value={String(settings.galleryPageSize)}
          disabled={settings.galleryPaging !== "paged"}
          onValueChange={(v) => save({ galleryPageSize: Number(v) as Settings["galleryPageSize"] })}
        >
          <SelectTrigger aria-label={t("settings.view.pageSize")} className="w-52">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {PAGE_SIZES.map((n) => (
              <SelectItem key={n} value={String(n)}>
                {t("settings.view.pageSizeOption", { n })}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </SettingRow>
      <CardRadioGroup
        title={t("settings.view.indexRail")}
        description={t("settings.view.indexRailHint")}
        value={settings.indexRail}
        options={railOptions}
        onChange={(indexRail) => save({ indexRail })}
        columnsClassName="grid-cols-3 sm:max-w-xl"
      />
    </div>
  )
}
