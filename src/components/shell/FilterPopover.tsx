import { ListFilter } from "lucide-react"
import { useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import type { MediaType } from "@/lib/ipc-contract"
import { dateCodec, FilterRange, numberCodec } from "./FilterRange"
import { FilterRatingRow } from "./FilterRatingRow"
import { UserFieldFilters } from "./UserFieldFilters"
import { activeFilterCount, useSearchStore } from "@/stores"

const MEDIA_TYPES: MediaType[] = ["image", "video", "audio"]
const MB = 1024 * 1024
const bytesCodec = numberCodec(MB)
const secondsCodec = numberCodec(1000)

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col !items-start gap-1">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      <div className="flex items-center gap-2">{children}</div>
    </div>
  )
}

function CheckRow({ id, label, checked, onChange }: { id: string; label: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <div className="flex items-center gap-2">
      <Checkbox id={id} checked={checked} onCheckedChange={(v) => onChange(v === true)} />
      <Label htmlFor={id} className="font-normal">
        {label}
      </Label>
    </div>
  )
}

/** 필터 Popover — 유형·오프라인/파일 없음·촬영일·크기·길이·즐겨찾기·별점. 변경 즉시 반영. */
export function FilterPopover() {
  const { t } = useTranslation()
  const filters = useSearchStore((s) => s.filters)
  const patch = useSearchStore((s) => s.patchFilters)
  const reset = useSearchStore((s) => s.resetFilters)
  const [resetKey, setResetKey] = useState(0)
  const count = activeFilterCount(filters)

  const toggleType = (type: MediaType, on: boolean) =>
    patch({ mediaTypes: on ? [...filters.mediaTypes, type] : filters.mediaTypes.filter((m) => m !== type) })

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="outline" aria-label={t("filter.label")}>
          <ListFilter aria-hidden="true" />
          {/* 상단 바 컨테이너 < 640px 에서는 라벨을 숨기고 아이콘 + 활성 개수만 (접근 가능한 이름은 aria-label) */}
          <span className="@max-[640px]:hidden">{t("filter.label")}</span>
          {count > 0 && (
            <Badge className="rounded-sm px-1.5" aria-label={t("filter.active", { count })}>
              {count}
            </Badge>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="relative flex max-h-(--radix-popover-content-available-height) w-96 flex-col gap-3 overflow-y-auto">
        <div key={resetKey} className="flex flex-col gap-3">
          <Row label={t("filter.mediaType")}>
            {MEDIA_TYPES.map((m) => (
              <CheckRow key={m} id={`f-type-${m}`} label={t(`asset.type.${m}`)} checked={filters.mediaTypes.includes(m)} onChange={(v) => toggleType(m, v)} />
            ))}
          </Row>
          <Row label={t("filter.availability")}>
            <CheckRow id="f-offline" label={t("filter.includeOffline")} checked={filters.includeOffline} onChange={(v) => patch({ includeOffline: v })} />
            <CheckRow id="f-missing" label={t("filter.includeMissing")} checked={filters.includeMissing} onChange={(v) => patch({ includeMissing: v })} />
          </Row>
          <CheckRow id="f-fav" label={t("filter.favoritesOnly")} checked={filters.favoritesOnly} onChange={(v) => patch({ favoritesOnly: v })} />
          <FilterRatingRow
            label={t("filter.rating")}
            min={filters.ratingMin}
            max={filters.ratingMax}
            onApply={(a, b) => patch({ ratingMin: a, ratingMax: b })}
          />
          <FilterRange
            label={t("filter.captured")}
            minLabel={t("filter.capturedFrom")}
            maxLabel={t("filter.capturedTo")}
            codec={dateCodec}
            min={filters.capturedFrom}
            max={filters.capturedTo}
            onApply={(a, b) => patch({ capturedFrom: a, capturedTo: b })}
          />
          <FilterRange
            label={t("filter.size")}
            minLabel={t("filter.sizeMin")}
            maxLabel={t("filter.sizeMax")}
            codec={bytesCodec}
            min={filters.sizeMin}
            max={filters.sizeMax}
            inputClassName="w-24"
            onApply={(a, b) => patch({ sizeMin: a, sizeMax: b })}
          />
          <FilterRange
            label={t("filter.duration")}
            minLabel={t("filter.durationMin")}
            maxLabel={t("filter.durationMax")}
            codec={secondsCodec}
            min={filters.durationMinMs}
            max={filters.durationMaxMs}
            inputClassName="w-24"
            onApply={(a, b) => patch({ durationMinMs: a, durationMaxMs: b })}
          />
          <UserFieldFilters />
        </div>
        <Button
          variant="ghost"
          size="sm"
          className="self-end"
          onClick={() => {
            reset()
            setResetKey((k) => k + 1)
          }}
        >
          {t("filter.reset")}
        </Button>
      </PopoverContent>
    </Popover>
  )
}
