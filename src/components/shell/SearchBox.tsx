import { useMutation, useQueryClient } from "@tanstack/react-query"
import { Search } from "lucide-react"
import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { qk } from "@/hooks/keys"
import { useRecentSearches } from "@/hooks/useLibrary"
import { useSettings } from "@/hooks/useSettings"
import { useApi } from "@/lib/api"
import { cn } from "@/lib/utils"
import { useSearchStore } from "@/stores"
import { SearchModeToggle } from "./SearchModeToggle"
import { SearchTagChips } from "./SearchTagChips"
import { TagSuggest } from "./TagSuggest"
import { MAX_TAG_CHIPS, useTagAutocomplete } from "./useTagAutocomplete"

/** D3: 250ms 디바운스로 키워드 검색 실행, 의미 검색은 임베딩 추론이 필요해 600ms. Enter 는 즉시 실행 + 최근 검색어 기록. */
export const SEARCH_DEBOUNCE_MS = 250
export const SEMANTIC_DEBOUNCE_MS = 600

export function SearchBox() {
  const { t } = useTranslation()
  const api = useApi()
  const qc = useQueryClient()
  const query = useSearchStore((s) => s.query)
  const filters = useSearchStore((s) => s.filters)
  const setQuery = useSearchStore((s) => s.setQuery)
  const mode = useSearchStore((s) => s.mode)
  const [text, setText] = useState(query)
  const [focused, setFocused] = useState(false)
  const recent = useRecentSearches()
  const tagAc = useTagAutocomplete({ text, setText, focused })
  // 제목은 편집이 켜져 있을 때만 검색 대상이다 (백엔드 규칙) — 설정을 받기 전에는 파일명 기준 문구
  const titleSearch = useSettings().data?.titleEditEnabled === true

  // 저장한 검색 등 외부에서 질의가 바뀌면 입력창에 반영
  useEffect(() => {
    setText((prev) => (prev.trim() === query ? prev : query))
  }, [query])

  // `#` 추천 팝업이 열려 있는 동안은 미완성 토큰(`#여`)으로 검색하지 않고 이전 결과를 유지한다
  const tagPicking = tagAc.popup
  useEffect(() => {
    if (tagPicking) return
    const id = setTimeout(() => setQuery(text.trim()), mode === "hybrid" ? SEMANTIC_DEBOUNCE_MS : SEARCH_DEBOUNCE_MS)
    return () => clearTimeout(id)
  }, [text, setQuery, mode, tagPicking])

  const record = useMutation({
    mutationFn: (q: string) => api.recordRecentSearch(q, filters),
    onSuccess: () => qc.invalidateQueries({ queryKey: qk.recentSearches }),
  })
  const clear = useMutation({
    mutationFn: () => api.clearRecentSearches(),
    onSuccess: () => qc.invalidateQueries({ queryKey: qk.recentSearches }),
  })

  const commit = (value: string) => {
    const q = value.trim()
    setText(value)
    setQuery(q)
    if (q) record.mutate(q)
  }

  const items = recent.data ?? []
  const showRecent = focused && text === "" && items.length > 0

  return (
    <div className="relative w-full min-w-0 max-w-xl flex-1">
      <div className="absolute top-1/2 left-1.5 z-10 -translate-y-1/2">
        <SearchModeToggle />
      </div>
      <Search
        aria-hidden="true"
        data-testid="search-icon"
        data-visible={focused ? "true" : "false"}
        className={`pointer-events-none absolute top-1/2 left-9 size-4 -translate-y-1/2 text-muted-foreground transition-opacity duration-150 motion-reduce:transition-none ${focused ? "opacity-100" : "opacity-0"}`}
      />
      {/* 칩이 입력과 한 줄에 놓이도록 필드 테두리는 래퍼가 그리고 입력은 테두리 없이 둔다 */}
      <div
        className={cn(
          "flex h-9 w-full min-w-0 items-center gap-1 rounded-md border border-input bg-transparent pr-2.5 shadow-xs transition-[color,box-shadow,padding] duration-150 focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/50 motion-reduce:transition-none dark:bg-input/30",
          focused ? "pl-15" : "pl-9"
        )}
      >
        <SearchTagChips chips={tagAc.chips} onRemove={tagAc.removeChip} />
        <Input
          ref={tagAc.inputRef}
          type="search"
          role="combobox"
          aria-autocomplete="list"
          aria-expanded={tagAc.popup}
          aria-controls={tagAc.popup && !tagAc.atCap && tagAc.items.length > 0 ? tagAc.listId : undefined}
          aria-activedescendant={tagAc.activeDescendant}
          value={text}
          placeholder={t(titleSearch ? "search.placeholderWithTitle" : "search.placeholder")}
          aria-label={t("search.label")}
          data-icon-visible={focused ? "true" : "false"}
          className="h-full min-w-0 flex-1 text-ellipsis rounded-none border-0 bg-transparent px-0 shadow-none focus-visible:ring-0 dark:bg-transparent"
          onChange={(e) => tagAc.onTextChange(e.target.value, e.target.selectionStart ?? e.target.value.length)}
          onSelect={(e) => tagAc.onCaretChange(e.currentTarget.selectionStart ?? 0)}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          onKeyDown={(e) => {
            if (tagAc.onKeyDown(e)) return
            if (e.key === "Enter") commit(text)
          }}
        />
      </div>
      {tagAc.popup && (
        <TagSuggest
          id={tagAc.listId}
          items={tagAc.items}
          activeIndex={tagAc.activeIndex}
          limitReached={tagAc.atCap}
          maxTags={MAX_TAG_CHIPS}
          onPick={tagAc.addPicked}
        />
      )}
      {showRecent && (
        // 포커스를 유지하려고 mousedown 기본 동작을 막는다
        <div
          onMouseDown={(e) => e.preventDefault()}
          className="absolute top-full left-0 z-30 mt-1 w-full rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-md"
        >
          <div className="flex items-center justify-between px-2 py-1">
            <span className="text-xs text-muted-foreground">{t("search.recent")}</span>
            <Button size="xs" variant="ghost" onClick={() => clear.mutate()}>
              {t("search.clearRecent")}
            </Button>
          </div>
          <ul aria-label={t("search.recent")}>
            {items.slice(0, 8).map((r) => (
              <li key={r.query}>
                <button
                  type="button"
                  onClick={() => commit(r.query)}
                  className="w-full truncate rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted"
                >
                  {r.query}
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}
