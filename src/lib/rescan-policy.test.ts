// Categories: Happy (T-ui-pol-01 roundtrip, 03 describe) / Validation (02 fixture parity, 04 draft) / Boundary (05 unchanged)
import { createInstance } from "i18next"
import { describe, expect, it } from "vitest"
import cases from "../../crates/mam-core/tests/fixtures/rescan_policy_cases.json"
import ko from "@/i18n/locales/ko.json"
import type { RescanPolicy, StoredRescanPolicy } from "./ipc-contract"
import {
  POLICY_DEFAULT,
  POLICY_UNKNOWN,
  describePolicy,
  draftFromPolicy,
  normalizeRescanPolicy,
  policyFromDraft,
  validateDraft,
  validateRescanPolicy,
} from "./rescan-policy"

async function makeT() {
  const i18n = createInstance()
  await i18n.init({ lng: "ko", resources: { ko: { translation: ko } }, interpolation: { escapeValue: false } })
  return i18n.t.bind(i18n)
}

const schedule = (days: string[], at = "03:00"): StoredRescanPolicy => ({ trigger: { kind: "schedule", days, at }, window: null, onStart: false })

describe("rescan policy", () => {
  it("T-ui-pol-01 [Happy] policy -> draft -> policy roundtrips for every known kind (window, onStart included)", () => {
    // Arrange
    const policies: RescanPolicy[] = [
      { trigger: { kind: "interval", hours: 12 }, window: { start: "22:00", end: "06:00" }, onStart: true },
      { trigger: { kind: "schedule", days: ["mon", "fri"], at: "03:30" }, window: null, onStart: false },
      { trigger: { kind: "manual" }, window: null, onStart: true },
    ]
    for (const p of policies) {
      // Act: 기준을 "기본값" 으로 두면 어떤 정책이든 변경으로 판정된다
      const draft = draftFromPolicy(p, 6)
      const back = policyFromDraft(draft, draftFromPolicy(null, 6))
      // Assert
      expect(validateDraft(draft)).toEqual({})
      expect(back).toEqual(normalizeRescanPolicy(p))
    }
  })

  it("T-ui-pol-02 [Validation] validateRescanPolicy returns exactly the Rust field path for every shared fixture case", () => {
    // Assert (스캐너가 0건을 읽는 회귀도 잡는다)
    expect(cases.length).toBeGreaterThanOrEqual(30)
    const mismatches = cases
      .map((c) => ({ input: c.input, want: c.field, got: validateRescanPolicy(c.input) }))
      .filter((r) => r.want !== r.got)
    expect(mismatches).toEqual([])
  })

  it("T-ui-pol-03 [Happy] describePolicy summarizes default / interval / weekdays / weekends / custom days / manual / unknown", async () => {
    // Arrange
    const t = await makeT()
    // Act / Assert
    expect(describePolicy(t, null, 6)).toBe("6시간마다 (기본)")
    expect(describePolicy(t, { trigger: { kind: "interval", hours: 2 } }, 6)).toBe("2시간마다")
    expect(describePolicy(t, schedule(["mon", "tue", "wed", "thu", "fri"]), 6)).toBe("평일 03:00")
    expect(describePolicy(t, schedule(["sat", "sun"]), 6)).toBe("주말 03:00")
    expect(describePolicy(t, schedule([]), 6)).toBe("매일 03:00")
    expect(describePolicy(t, schedule(["mon", "wed"]), 6)).toBe("월 수 03:00")
    expect(describePolicy(t, { trigger: { kind: "manual" }, window: { start: "22:00", end: "06:00" }, onStart: true }, 6)).toBe(
      "수동, 22:00~06:00에만, 시작 시 1회",
    )
    expect(describePolicy(t, { trigger: { kind: "weekly" } }, 6)).toBe("알 수 없는 정책")
  })

  it("T-ui-pol-04 [Validation] validateDraft flags range, time and equal-window errors only for visible inputs", () => {
    // Arrange
    const base = draftFromPolicy({ trigger: { kind: "interval", hours: 6 } }, 6)
    // Act / Assert
    expect(validateDraft({ ...base, hours: "0" })).toEqual({ hours: "range" })
    expect(validateDraft({ ...base, hours: "721" })).toEqual({ hours: "range" })
    expect(validateDraft({ ...base, hours: "1.5" })).toEqual({ hours: "range" })
    expect(validateDraft({ ...base, hours: "" })).toEqual({ hours: "range" })
    expect(validateDraft({ ...base, kind: "schedule", at: "25:00" })).toEqual({ at: "time" })
    expect(validateDraft({ ...base, windowEnabled: true, windowStart: "22:00", windowEnd: "22:00" })).toEqual({ window: "same" })
    expect(validateDraft({ ...base, windowEnabled: true, windowStart: "22:00", windowEnd: "06:00" })).toEqual({})
    // 수동에는 시간대 칸이 없으므로 잘못된 값이 남아 있어도 오류가 아니다
    expect(validateDraft({ ...base, kind: "manual", windowEnabled: true, windowStart: "99:99" })).toEqual({})
    expect(validateDraft({ ...base, kind: "manual", hours: "0" })).toEqual({})
  })

  it("T-ui-pol-05 [Boundary] policyFromDraft: untouched or unknown = unchanged, default after a policy = null, days normalize", () => {
    // Arrange
    const stored: StoredRescanPolicy = { trigger: { kind: "interval", hours: 4 }, window: null, onStart: false }
    const initial = draftFromPolicy(stored, 6)
    const unknown = draftFromPolicy({ trigger: { kind: "weekly" } }, 6)
    // Assert
    expect(unknown.kind).toBe(POLICY_UNKNOWN)
    expect(policyFromDraft(initial, initial)).toBe("unchanged")
    expect(policyFromDraft(unknown, unknown)).toBe("unchanged")
    expect(policyFromDraft({ ...initial, kind: POLICY_DEFAULT }, initial)).toBeNull()
    expect(policyFromDraft({ ...initial, kind: "schedule", days: ["sun", "mon", "mon"], at: "01:00" }, initial)).toEqual({
      trigger: { kind: "schedule", days: ["mon", "sun"], at: "01:00" },
      window: null,
      onStart: false,
    })
    expect(
      policyFromDraft({ ...initial, kind: "schedule", days: ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] }, initial),
    ).toMatchObject({ trigger: { days: [] } })
  })
})
