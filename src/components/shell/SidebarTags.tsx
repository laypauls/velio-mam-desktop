import { Tag as TagIcon, Tags } from "lucide-react"
import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useTags } from "@/hooks/useLibrary"
import { useActionsStore, useSearchStore } from "@/stores"
import { Group, NavItem } from "./SidebarParts"
import { MAX_TAG_CHIPS } from "./useTagAutocomplete"

/** 한 번에 그리는 태그 수 — 그보다 많으면 검색으로 좁히게 안내한다(전체를 DOM 에 올리지 않는다) */
export const TAG_LIST_LIMIT = 300

/** 태그 선택 규칙 — 클릭 = 그 태그만(`tagIds` 교체), Ctrl/⌘+클릭 = 추가(AND, 상한 `MAX_TAG_CHIPS`), 이미 고른 태그 클릭 = 해제. */
export function nextTagIds(current: number[], id: number, additive: boolean): number[] | "limit" {
  if (current.includes(id)) return current.filter((x) => x !== id)
  if (!additive) return [id]
  return current.length >= MAX_TAG_CHIPS ? "limit" : [...current, id]
}

/** "태그" 섹션 패널 — 평면 태그 목록(이름 + 개수), 검색, 태그 관리. 선택은 검색창 태그 칩과 같은 `filters.tagIds` 다. */
export function SidebarTags() {
  const { t } = useTranslation()
  const tags = useTags()
  const tagIds = useSearchStore((s) => s.filters.tagIds)
  const patchFilters = useSearchStore((s) => s.patchFilters)
  const setTagManagerOpen = useActionsStore((s) => s.setTagManagerOpen)
  const [text, setText] = useState("")

  const needle = text.trim().toLowerCase()
  const matched = useMemo(() => (tags.data ?? []).filter((x) => needle === "" || x.name.toLowerCase().includes(needle)), [tags.data, needle])
  const shown = matched.slice(0, TAG_LIST_LIMIT)

  const onPick = (id: number, additive: boolean) => {
    const next = nextTagIds(tagIds, id, additive)
    if (next === "limit") toast.info(t("search.tags.limit", { max: MAX_TAG_CHIPS }))
    else patchFilters({ tagIds: next })
  }

  return (
    <Group
      title={t("sidebar.tags")}
      action={
        <Button size="icon-xs" variant="ghost" aria-label={t("sidebar.manageTags")} title={t("sidebar.manageTags")} onClick={() => setTagManagerOpen(true)}>
          <Tags aria-hidden="true" />
        </Button>
      }
    >
      <Input type="search" value={text} onChange={(e) => setText(e.target.value)} aria-label={t("sidebar.tagSearch")} placeholder={t("sidebar.tagSearch")} className="h-8" />
      {tags.isError ? (
        <p role="alert" className="px-2 text-xs break-keep text-destructive">
          {t("sidebar.tagsError")}
        </p>
      ) : tags.data && shown.length === 0 ? (
        <p className="px-2 text-xs break-keep text-muted-foreground">{t(tags.data.length === 0 ? "sidebar.tagsEmpty" : "sidebar.tagsNoMatch")}</p>
      ) : (
        <ul>
          {shown.map((tag) => (
            <NavItem key={tag.tagId} icon={TagIcon} label={tag.name} count={tag.assetCount} active={tagIds.includes(tag.tagId)} onClick={(e) => onPick(tag.tagId, e.ctrlKey || e.metaKey)} />
          ))}
        </ul>
      )}
      {matched.length > shown.length && <p className="px-2 text-xs break-keep text-muted-foreground">{t("sidebar.tagsMore", { shown: shown.length })}</p>}
    </Group>
  )
}
