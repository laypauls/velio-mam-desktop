// Categories: Happy / Boundary (empty token, caret position, abc#x, whitespace) / Business rule (NFC, case, exclusion, zero-count, ranking)
import { describe, expect, it } from "vitest"
import type { Tag } from "@/lib/ipc-contract"
import { extractTagToken, findExactTag, rankTagSuggestions, removeTagToken } from "./tagToken"

const tag = (tagId: number, name: string, assetCount: number): Tag => ({ tagId, name, assetCount })

describe("extractTagToken", () => {
  it("T-ui-tagtok-01 [Happy] extracts the # token ending at the caret", () => {
    // Arrange + Act
    const tok = extractTagToken("노을 #여", 5)
    // Assert
    expect(tok).toEqual({ start: 3, end: 5, query: "여" })
  })

  it("T-ui-tagtok-02 [Boundary] a lone # at the start is an empty-query token", () => {
    expect(extractTagToken("#", 1)).toEqual({ start: 0, end: 1, query: "" })
  })

  it("T-ui-tagtok-03 [Boundary] # glued to a word (abc#x) is plain text", () => {
    expect(extractTagToken("abc#x", 5)).toBeNull()
  })

  it("T-ui-tagtok-04 [Boundary] caret in the middle only uses text before the caret", () => {
    // Arrange + Act
    const tok = extractTagToken("#여름 바다", 2)
    // Assert
    expect(tok).toEqual({ start: 0, end: 2, query: "여" })
  })

  it("T-ui-tagtok-05 [Boundary] token ends at whitespace: caret in a later word or right after a space gives no token", () => {
    expect(extractTagToken("#여름 바다", 6)).toBeNull()
    expect(extractTagToken("#여름 ", 4)).toBeNull()
  })

  it("T-ui-tagtok-06 [Boundary] caret before the # or out of range yields no token and does not throw", () => {
    expect(extractTagToken("a #b", 2)).toBeNull()
    expect(extractTagToken("", 0)).toBeNull()
    expect(extractTagToken("#a", 99)).toEqual({ start: 0, end: 2, query: "a" })
    expect(extractTagToken("#a", -3)).toBeNull()
  })
})

describe("removeTagToken", () => {
  it("T-ui-tagtok-07 [Happy] removes a trailing token and the separating space", () => {
    // Arrange
    const tok = extractTagToken("노을 #여", 5)!
    // Act + Assert
    expect(removeTagToken("노을 #여", tok)).toEqual({ text: "노을", caret: 2 })
  })

  it("T-ui-tagtok-08 [Boundary] removing a mid-text token keeps a single space and puts the caret at the join", () => {
    const tok = extractTagToken("a #b c", 4)!
    expect(removeTagToken("a #b c", tok)).toEqual({ text: "a c", caret: 1 })
  })

  it("T-ui-tagtok-09 [Boundary] removing the only token leaves an empty string", () => {
    const tok = extractTagToken("#a", 2)!
    expect(removeTagToken("#a", tok)).toEqual({ text: "", caret: 0 })
  })
})

describe("rankTagSuggestions", () => {
  const all = [
    tag(1, "가족여행", 12),
    tag(2, "여행", 57),
    tag(3, "여름", 128),
    tag(4, "여우", 3),
    tag(5, "바다", 400),
    tag(6, "Summer", 10),
    tag(7, "여백", 0),
  ]

  it("T-ui-tagtok-10 [Business rule] prefix matches come first, then substring, each by assetCount desc", () => {
    // Act
    const out = rankTagSuggestions(all, "여", [])
    // Assert
    expect(out.map((x) => x.name)).toEqual(["여름", "여행", "여우", "가족여행"])
  })

  it("T-ui-tagtok-11 [Boundary] an empty token lists the most used tags first, capped at 8", () => {
    // Arrange
    const many = Array.from({ length: 12 }, (_, i) => tag(100 + i, `t${i}`, i + 1))
    // Act
    const out = rankTagSuggestions(many, "", [])
    // Assert
    expect(out).toHaveLength(8)
    expect(out[0].name).toBe("t11")
  })

  it("T-ui-tagtok-12 [Business rule] matching ignores case and Unicode normalization form", () => {
    // Arrange: NFD 로 분해된 한글 입력
    const nfd = "여름".normalize("NFD")
    // Act + Assert
    expect(rankTagSuggestions(all, "SUMMER", []).map((x) => x.name)).toEqual(["Summer"])
    expect(rankTagSuggestions(all, nfd, []).map((x) => x.name)).toEqual(["여름"])
  })

  it("T-ui-tagtok-13 [Business rule] already selected tags are excluded", () => {
    expect(rankTagSuggestions(all, "여", [3]).map((x) => x.name)).not.toContain("여름")
  })

  it("T-ui-tagtok-14 [Business rule] tags with assetCount 0 are hidden", () => {
    expect(rankTagSuggestions(all, "여", []).map((x) => x.name)).not.toContain("여백")
  })

  it("T-ui-tagtok-15 [Boundary] no match and an empty list both give an empty result", () => {
    expect(rankTagSuggestions(all, "zzz", [])).toEqual([])
    expect(rankTagSuggestions([], "", [])).toEqual([])
  })
})

describe("findExactTag", () => {
  const all = [tag(1, "Summer", 5), tag(2, "여름", 5), tag(3, "빈태그", 0)]

  it("T-ui-tagtok-16 [Happy] finds an exact name ignoring case and NFC", () => {
    expect(findExactTag(all, "summer", [])?.tagId).toBe(1)
    expect(findExactTag(all, "빈태그", [])?.tagId).toBe(3) // 사용 0건이어도 이름이 정확히 같으면 찾는다
    expect(findExactTag(all, "여름".normalize("NFD"), [])?.tagId).toBe(2)
  })

  it("T-ui-tagtok-17 [Validation] partial names, selected tags and empty tokens do not match", () => {
    expect(findExactTag(all, "summ", [])).toBeNull()
    expect(findExactTag(all, "Summer", [1])).toBeNull()
    expect(findExactTag(all, "빈태그", [3])).toBeNull()
    expect(findExactTag(all, "", [])).toBeNull()
  })
})
