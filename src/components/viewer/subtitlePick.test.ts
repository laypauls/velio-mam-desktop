// Categories: Business rule (T-ui-sub-01) / Boundary (T-ui-sub-01 empty, single, all-unknown)
import { describe, expect, it } from "vitest"
import type { SubtitleTrack } from "@/lib/ipc-contract"
import { pickTrack, type PickInput } from "./subtitlePick"

const t = (id: string, language: string | null): SubtitleTrack => ({ id, language, format: "srt", fileName: id })
const BASE: PickInput = { visible: true, sessionLanguage: undefined, defaultLanguage: "ko" }

describe("pickTrack (D4·D7·D10)", () => {
  it("T-ui-sub-01 [Business rule] follows the rule order: visible → session → default off → default language → single unknown → first", () => {
    // Arrange
    const en = t("en", "en")
    const ko = t("ko", "ko")
    const ja = t("ja", "ja")
    const unk = t("unk", null)
    // Act + Assert: 표시 꺼짐은 모든 것보다 우선
    expect(pickTrack([ko, en], { ...BASE, visible: false })).toBeNull()
    expect(pickTrack([ko, en], { visible: false, sessionLanguage: "en", defaultLanguage: "ko" })).toBeNull()
    // 트랙 없음
    expect(pickTrack([], BASE)).toBeNull()
    expect(pickTrack([], { ...BASE, sessionLanguage: "en" })).toBeNull()
    // 세션 언어가 기본 언어보다 먼저
    expect(pickTrack([ko, en], { ...BASE, sessionLanguage: "en" })).toBe("en")
    expect(pickTrack([ko, t("en2", "en"), en], { ...BASE, sessionLanguage: "en" })).toBe("en2") // 같은 언어의 첫 트랙
    // 세션 언어 트랙이 없으면 기본 언어 규칙으로 내려간다
    expect(pickTrack([ko, ja], { ...BASE, sessionLanguage: "en" })).toBe("ko")
    // 세션 null = 언어를 알 수 없는 트랙을 골랐다 → 알 수 없는 트랙이 정확히 1개일 때만
    expect(pickTrack([ko, unk], { ...BASE, sessionLanguage: null })).toBe("unk")
    expect(pickTrack([ko, unk, t("unk2", null)], { ...BASE, sessionLanguage: null })).toBe("ko")
    // 기본 언어 = 끄기: 세션 언어가 맞지 않으면 자막 없이 시작
    expect(pickTrack([ko, en], { ...BASE, defaultLanguage: null })).toBeNull()
    expect(pickTrack([ko, en], { visible: true, sessionLanguage: "en", defaultLanguage: null })).toBe("en")
    expect(pickTrack([ko], { visible: true, sessionLanguage: "ja", defaultLanguage: null })).toBeNull()
    // D4 ①
    expect(pickTrack([en, ja], { ...BASE, defaultLanguage: "ja" })).toBe("ja")
    // D4 ② 기본 언어 트랙이 없고 알 수 없는 트랙이 1개뿐이면 그것
    expect(pickTrack([en, unk], BASE)).toBe("unk")
    // D4 ③ 알 수 없는 트랙이 여러 개면 첫 트랙 (Rust 정렬 순)
    expect(pickTrack([en, unk, t("unk2", null)], BASE)).toBe("en")
    expect(pickTrack([unk, t("unk2", null)], BASE)).toBe("unk")
    // 트랙 1개
    expect(pickTrack([en], BASE)).toBe("en")
  })
})
