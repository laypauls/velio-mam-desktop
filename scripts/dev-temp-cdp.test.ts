// 분류: Happy / Boundary / Validation
import { describe, expect, it } from "vitest"
import { DEFAULT_CDP_PORT, hasCdpFlag, mergeBrowserArgs, resolveCdpPort } from "./dev-temp-cdp.mjs"

describe("dev-temp-cdp resolveCdpPort", () => {
  it("T-devcdp-01 [Happy] 미지정이면 기본 19333", () => {
    expect(resolveCdpPort(undefined)).toBe(19333)
    expect(DEFAULT_CDP_PORT).toBe(19333)
  })
  it("T-devcdp-02 [Happy] 지정한 포트로 재지정", () => {
    expect(resolveCdpPort("9444")).toBe(9444)
  })
  it("T-devcdp-03 [Boundary] 0 은 끔(null)", () => {
    expect(resolveCdpPort("0")).toBeNull()
  })
  it("T-devcdp-04 [Boundary] 빈 문자열은 기본값, 1 과 65535 는 허용", () => {
    expect(resolveCdpPort("")).toBe(19333)
    expect(resolveCdpPort("1")).toBe(1)
    expect(resolveCdpPort("65535")).toBe(65535)
  })
  it.each(["abc", "70000", "-1", "65536", "1.5", "9333x"])("T-devcdp-05 [Validation] %s 거부", (v) => {
    expect(() => resolveCdpPort(v)).toThrow(/MAM_DEV_CDP_PORT/)
  })
})

describe("dev-temp-cdp mergeBrowserArgs", () => {
  it("T-devcdp-06 [Happy] 기존 값이 없으면 포트 인자만", () => {
    expect(mergeBrowserArgs(undefined, 19333)).toBe("--remote-debugging-port=19333")
    expect(mergeBrowserArgs("  ", 19333)).toBe("--remote-debugging-port=19333")
  })
  it("T-devcdp-07 [Happy] 기존 인자 뒤에 공백으로 덧붙임", () => {
    expect(mergeBrowserArgs("--foo=1", 19333)).toBe("--foo=1 --remote-debugging-port=19333")
  })
  it("T-devcdp-08 [Business rule] 기존 --remote-debugging-port 는 그대로 유지", () => {
    const existing = "--remote-debugging-port=9555 --foo"
    expect(hasCdpFlag(existing)).toBe(true)
    expect(mergeBrowserArgs(existing, 19333)).toBe(existing)
    expect(hasCdpFlag("--foo")).toBe(false)
  })
})
