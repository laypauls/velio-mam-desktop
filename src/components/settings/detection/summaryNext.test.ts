// Categories: Happy (01-03) / Boundary (04)
import { describe, expect, it } from "vitest"
import i18n from "@/i18n"
import type { ManagedFolder } from "@/lib/ipc-contract"
import { rescanNextText } from "./summary"

const t = i18n.t.bind(i18n)
const NOW = new Date(2026, 10, 22, 10, 0).getTime()
const at = (m: number, d: number, h: number, min = 0) => ({ nextRescanAt: new Date(2026, m, d, h, min).getTime() }) as ManagedFolder

describe("rescanNextText (table row, short format)", () => {
  it("T-ui-rescannext-01 [Happy] same day reads 오늘 + time", () => {
    const text = rescanNextText(t, at(10, 22, 15, 40), NOW)
    expect(text).toMatch(/^다음 오늘 .*3:40|^다음 오늘 .*15:40/)
  })
  it("T-ui-rescannext-02 [Happy] next day reads 내일 + time, even across midnight", () => {
    expect(rescanNextText(t, at(10, 23, 0, 5), new Date(2026, 10, 22, 23, 59).getTime())).toMatch(/^다음 내일 /)
  })
  it("T-ui-rescannext-03 [Happy] later dates show month/day + time without the year", () => {
    const text = rescanNextText(t, at(10, 25, 15, 40), NOW) ?? ""
    expect(text).toMatch(/^다음 11월 25일|^다음 11\. 25\./)
    expect(text).not.toContain("2026")
    expect(text).not.toMatch(/오늘|내일/)
  })
  it("T-ui-rescannext-04 [Boundary] no schedule is null", () => {
    expect(rescanNextText(t, { nextRescanAt: null } as ManagedFolder, NOW)).toBeNull()
  })
})
