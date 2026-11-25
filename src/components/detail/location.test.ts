// Categories: Happy (T-ui-detail-14a) / Boundary (14b root-level, 14c case, 14d slashes) / Validation (14e outside root, 14f no alias)
import { describe, expect, it } from "vitest"
import { locationText } from "./location"

const folder = { alias: "촬영원본", path: "E:\\Media\\Shoots" }

describe("locationText", () => {
  it("T-ui-detail-14a [Happy] alias › sub-path without the file name", () => {
    expect(locationText("E:\\Media\\Shoots\\2024\\Jeju\\a.mp4", folder)).toBe("촬영원본 › 2024\\Jeju")
  })
  it("T-ui-detail-14b [Boundary] file directly under the folder root shows only the alias", () => {
    expect(locationText("E:\\Media\\Shoots\\a.mp4", folder)).toBe("촬영원본")
  })
  it("T-ui-detail-14c [Boundary] root matching ignores case (Windows)", () => {
    expect(locationText("e:\\media\\shoots\\x\\a.mp4", folder)).toBe("촬영원본 › x")
  })
  it("T-ui-detail-14d [Boundary] forward slashes are accepted", () => {
    expect(locationText("E:/Media/Shoots/x/y/a.mp4", folder)).toBe("촬영원본 › x\\y")
  })
  it("T-ui-detail-14e [Validation] a path outside the folder root falls back to the full path", () => {
    expect(locationText("D:\\Other\\a.mp4", folder)).toBe("D:\\Other\\a.mp4")
  })
  it("T-ui-detail-14f [Business rule] no alias or unknown folder shows the full path", () => {
    const path = "E:\\Media\\Shoots\\a.mp4"
    expect(locationText(path, { alias: null, path: folder.path })).toBe(path)
    expect(locationText(path, null)).toBe(path)
  })
})
