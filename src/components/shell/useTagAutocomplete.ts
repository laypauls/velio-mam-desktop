import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"
import { useTags } from "@/hooks/useLibrary"
import type { Tag } from "@/lib/ipc-contract"
import { extractTagToken, findExactTag, rankTagSuggestions, removeTagToken, type TagToken } from "@/lib/tagToken"
import { useSearchStore } from "@/stores"
import { tagOptionId } from "./TagSuggest"
import type { TagChip } from "./SearchTagChips"

/** 백엔드 `MAX_TAG_FILTERS` (src-tauri/src/search_service.rs) 와 같아야 한다. */
export const MAX_TAG_CHIPS = 50

interface Args {
  text: string
  setText: (text: string) => void
  focused: boolean
}

/** 검색창 `#태그` 자동완성 — 칩(`filters.tagIds`)·추천 팝업·키보드 처리를 한곳에 모은다. */
export function useTagAutocomplete({ text, setText, focused }: Args) {
  const { t } = useTranslation()
  const tags = useTags()
  const tagIds = useSearchStore((s) => s.filters.tagIds)
  const patchFilters = useSearchStore((s) => s.patchFilters)
  const listId = useId()
  const inputRef = useRef<HTMLInputElement>(null)
  const pendingCaret = useRef<number | null>(null)
  const [caret, setCaret] = useState(0)
  const [dismissed, setDismissed] = useState(false)
  const [active, setActive] = useState(0)

  const list = tags.data
  const atCap = tagIds.length >= MAX_TAG_CHIPS
  const token: TagToken | null = focused && list ? extractTagToken(text, caret) : null
  const open = token !== null && !dismissed
  const items = useMemo<Tag[]>(
    () => (open && !atCap && list && token ? rankTagSuggestions(list, token.query, tagIds) : []),
    [open, atCap, list, token, tagIds]
  )
  const activeIndex = Math.min(active, Math.max(items.length - 1, 0))
  const hasItems = open && items.length > 0

  // 삭제된 태그 ID 는 필터에서 제거한다 — 남겨두면 결과가 0건이 된다 (목록을 정상 수신한 경우에만)
  useEffect(() => {
    if (!tags.isSuccess || tags.isFetching) return
    const known = new Set(tags.data.map((x) => x.tagId))
    const kept = tagIds.filter((id) => known.has(id))
    if (kept.length !== tagIds.length) patchFilters({ tagIds: kept })
  }, [tags.isSuccess, tags.isFetching, tags.data, tagIds, patchFilters])

  useLayoutEffect(() => {
    const pos = pendingCaret.current
    if (pos === null) return
    pendingCaret.current = null
    inputRef.current?.setSelectionRange(pos, pos)
    setCaret(pos)
  }, [text])

  const chips: TagChip[] = tagIds.map((id) => ({
    tagId: id,
    name: list?.find((x) => x.tagId === id)?.name ?? t("search.tags.unknown", { id }),
  }))

  const addTag = (tag: Tag, tok: TagToken) => {
    const next = removeTagToken(text, tok)
    pendingCaret.current = next.caret
    setText(next.text)
    setActive(0)
    patchFilters({ tagIds: [...tagIds, tag.tagId] })
  }

  const removeChip = (tagId: number) => patchFilters({ tagIds: tagIds.filter((id) => id !== tagId) })

  const onTextChange = (value: string, caretPos: number) => {
    setText(value)
    setCaret(caretPos)
    setDismissed(false)
    setActive(0)
  }

  /** 처리했으면 true — 호출 측은 자신의 Enter(검색 확정) 처리를 건너뛴다. */
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>): boolean => {
    const composing = e.nativeEvent.isComposing
    if (e.key === "Backspace") {
      if (text === "" && tagIds.length > 0 && !composing) {
        e.preventDefault()
        removeChip(tagIds[tagIds.length - 1])
        return true
      }
      return false
    }
    if (e.key === "Escape") {
      if (!open) return false
      // type="search" 입력은 Esc 기본 동작이 값을 비운다 — 팝업만 닫고 토큰 텍스트는 유지
      e.preventDefault()
      setDismissed(true)
      return true
    }
    if (e.key !== "Enter" && e.key !== "Tab" && e.key !== "ArrowDown" && e.key !== "ArrowUp") return false
    // 조합 중 Enter 는 추천이 떠 있을 때만 삼킨다(선택은 조합 확정 후). 그 외에는 기존처럼 검색 확정으로 흘려보낸다
    if (composing) return e.key === "Enter" && hasItems
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (!hasItems) return false
      e.preventDefault()
      setActive((activeIndex + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length)
      return true
    }
    if (hasItems && token) {
      e.preventDefault()
      addTag(items[activeIndex], token)
      return true
    }
    // 일치 항목 없음 + Enter: 정확히 같은 이름의 태그면 칩으로, 아니면 일반 텍스트 검색으로 둔다
    if (e.key === "Enter" && open && !atCap && list && token) {
      const exact = findExactTag(list, token.query, tagIds)
      if (exact) {
        e.preventDefault()
        addTag(exact, token)
        return true
      }
    }
    return false
  }

  const popup = open && (atCap || list !== undefined)
  return {
    inputRef,
    listId,
    chips,
    removeChip,
    popup,
    items,
    activeIndex,
    atCap,
    activeDescendant: hasItems ? tagOptionId(listId, items[activeIndex].tagId) : undefined,
    addPicked: (tag: Tag) => token && addTag(tag, token),
    onTextChange,
    onCaretChange: (pos: number) => setCaret(pos),
    onKeyDown,
  }
}
