import { X } from "lucide-react"
import { useTranslation } from "react-i18next"

export interface TagChip {
  tagId: number
  name: string
}

interface Props {
  chips: readonly TagChip[]
  onRemove: (tagId: number) => void
}

/** 검색창 안에 놓이는 선택된 태그 칩 목록 — 폭을 넘으면 가로 스크롤(스크롤바 숨김). */
export function SearchTagChips({ chips, onRemove }: Props) {
  const { t } = useTranslation()
  if (chips.length === 0) return null
  return (
    <ul
      aria-label={t("search.tags.chips")}
      className="scrollbar-hidden relative flex max-w-[60%] shrink-0 items-center gap-1 overflow-x-auto"
    >
      {chips.map((c) => (
        <li
          key={c.tagId}
          className="flex shrink-0 items-center gap-0.5 rounded-sm bg-secondary py-0.5 pr-0.5 pl-1.5 text-xs break-keep text-secondary-foreground"
        >
          <span>#{c.name}</span>
          <button
            type="button"
            aria-label={t("search.tags.remove", { name: c.name })}
            onClick={() => onRemove(c.tagId)}
            className="rounded-sm p-0.5 hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
          >
            <X aria-hidden="true" className="size-3" />
          </button>
        </li>
      ))}
    </ul>
  )
}
