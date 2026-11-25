// Categories: Happy (T-ui-alias-10a, T-ui-folderdisp-01/03) / Boundary (T-ui-alias-10b, T-ui-folderdisp-02)
import { describe, expect, it } from "vitest"
import { folderDisplayName, folderLabel, splitPathTail } from "./folders"

describe("folderLabel", () => {
  it("T-ui-alias-10a [Happy] returns the alias when set", () => {
    expect(folderLabel({ alias: "NAS 영상", path: "\\\\nas\\video" })).toBe("NAS 영상")
  })

  it("T-ui-alias-10b [Boundary] falls back to the path when the alias is null", () => {
    expect(folderLabel({ alias: null, path: "D:\\Photos" })).toBe("D:\\Photos")
  })
})

describe("folderDisplayName / splitPathTail", () => {
  it("T-ui-folderdisp-01 [Happy] alias, else last segment (both separators, trailing separator)", () => {
    expect(folderDisplayName({ alias: "별칭", path: "D:\\a\\b" })).toBe("별칭")
    expect(folderDisplayName({ alias: null, path: "D:\\a\\b" })).toBe("b")
    expect(folderDisplayName({ alias: null, path: "D:/a/b/" })).toBe("b")
    expect(folderDisplayName({ alias: null, path: "\\\\nas\\share\\영화\\" })).toBe("영화")
  })

  it("T-ui-folderdisp-02 [Boundary] roots fall back to the full path", () => {
    for (const path of ["D:\\", "D:", "\\\\server\\share", "\\\\server\\share\\", "/"]) {
      expect(folderDisplayName({ alias: null, path })).toBe(path)
      expect(splitPathTail(path)).toEqual({ head: path, tail: "" })
    }
  })

  it("T-ui-folderdisp-03 [Happy] split keeps the last separator in the tail so head + tail rebuilds the path", () => {
    expect(splitPathTail("D:\\a\\b")).toEqual({ head: "D:\\a", tail: "\\b" })
    expect(splitPathTail("\\\\nas\\share\\x\\y")).toEqual({ head: "\\\\nas\\share\\x", tail: "\\y" })
  })
})
