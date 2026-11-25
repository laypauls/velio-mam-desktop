import { useTranslation } from "react-i18next"
import type { Tag } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"

interface Props {
  id: string
  items: readonly Tag[]
  activeIndex: number
  /** 상한 도달 시 목록 대신 안내만 보인다 */
  limitReached: boolean
  maxTags: number
  onPick: (tag: Tag) => void
}

export const tagOptionId = (listId: string, tagId: number) => `${listId}-opt-${tagId}`

/** `#` 토큰 추천 팝업 — 입력 아래 absolute, mousedown 기본 동작을 막아 입력 포커스를 유지한다. */
export function TagSuggest({ id, items, activeIndex, limitReached, maxTags, onPick }: Props) {
  const { t } = useTranslation()
  return (
    <div
      onMouseDown={(e) => e.preventDefault()}
      className="absolute top-full left-0 z-30 mt-1 w-full rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-md"
    >
      {limitReached ? (
        <p role="status" className="px-2 py-1.5 text-sm break-keep text-muted-foreground">
          {t("search.tags.limit", { max: maxTags })}
        </p>
      ) : items.length === 0 ? (
        <p role="status" className="px-2 py-1.5 text-sm break-keep text-muted-foreground">
          {t("search.tags.empty")}
        </p>
      ) : (
        <ul id={id} role="listbox" aria-label={t("search.tags.suggestions")}>
          {items.map((tag, i) => (
            <li
              key={tag.tagId}
              id={tagOptionId(id, tag.tagId)}
              role="option"
              aria-selected={i === activeIndex}
              onClick={() => onPick(tag)}
              className={cn(
                "flex cursor-pointer items-center justify-between gap-2 rounded-md px-2 py-1.5 text-sm",
                i === activeIndex ? "bg-accent text-accent-foreground" : "hover:bg-muted"
              )}
            >
              <span className="truncate">#{tag.name}</span>
              <span className="shrink-0 text-xs text-muted-foreground">{tag.assetCount}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
