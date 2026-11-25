import type { Tag } from "@/lib/ipc-contract"

/** 검색창 `#태그` 자동완성의 순수 로직 — 커서 기준 토큰 추출·치환과 추천 정렬. */

/** 추천 목록 최대 개수 */
export const MAX_TAG_SUGGESTIONS = 8

/** 커서 앞 `#` 토큰. `start` = `#` 위치, `end` = 커서 위치(배타), `query` = `#` 뒤 ~ 커서 앞 */
export interface TagToken {
  start: number
  end: number
  query: string
}

const WHITESPACE = /\s/

/** 커서가 속한 단어(공백 구분)가 `#` 로 시작할 때만 토큰을 돌려준다. `abc#x` 는 일반 텍스트. */
export function extractTagToken(text: string, caret: number): TagToken | null {
  const end = Math.max(0, Math.min(caret, text.length))
  let start = end
  while (start > 0 && !WHITESPACE.test(text[start - 1])) start--
  if (text[start] !== "#" || start >= end) return null
  return { start, end, query: text.slice(start + 1, end) }
}

/** 토큰을 텍스트에서 지운다. 양옆이 모두 공백이면 하나만 남겨 공백이 겹치지 않게 한다. */
export function removeTagToken(text: string, token: TagToken): { text: string; caret: number } {
  let before = text.slice(0, token.start)
  const after = text.slice(token.end)
  if (before !== "" && WHITESPACE.test(before[before.length - 1]) && (after === "" || WHITESPACE.test(after[0]))) {
    before = before.slice(0, -1)
  }
  return { text: before + after, caret: before.length }
}

export function normalizeTagName(s: string): string {
  return s.normalize("NFC").toLowerCase()
}

function candidates(tags: readonly Tag[], selectedIds: readonly number[]): Tag[] {
  const selected = new Set(selectedIds)
  return tags.filter((t) => t.assetCount > 0 && !selected.has(t.tagId))
}

const byCountDesc = (a: Tag, b: Tag) => b.assetCount - a.assetCount || a.name.localeCompare(b.name)

/** 접두 일치 → 부분 일치 순, 각 그룹은 assetCount 내림차순. 빈 토큰이면 사용 많은 순. 선택된 태그·사용 0건 태그 제외. */
export function rankTagSuggestions(
  tags: readonly Tag[],
  query: string,
  selectedIds: readonly number[],
  limit = MAX_TAG_SUGGESTIONS
): Tag[] {
  const pool = candidates(tags, selectedIds)
  const needle = normalizeTagName(query)
  if (needle === "") return pool.sort(byCountDesc).slice(0, limit)
  const prefix: Tag[] = []
  const partial: Tag[] = []
  for (const tag of pool) {
    const name = normalizeTagName(tag.name)
    if (name.startsWith(needle)) prefix.push(tag)
    else if (name.includes(needle)) partial.push(tag)
  }
  return [...prefix.sort(byCountDesc), ...partial.sort(byCountDesc)].slice(0, limit)
}

/** 토큰 이름과 정확히 일치(NFC·대소문자 무시)하는 미선택 태그 — 없으면 null. 사용 0건 태그도 포함한다(추천에서만 숨김). */
export function findExactTag(tags: readonly Tag[], query: string, selectedIds: readonly number[]): Tag | null {
  const needle = normalizeTagName(query)
  if (needle === "") return null
  const selected = new Set(selectedIds)
  return tags.find((t) => !selected.has(t.tagId) && normalizeTagName(t.name) === needle) ?? null
}
