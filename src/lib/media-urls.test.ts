// Categories: Happy / Validation / Boundary (URL 형태 계약)
import { afterEach, describe, expect, it, vi } from "vitest"
import { MEDIA_SCHEME, STRIP_FRAMES, THUMB_SCHEME, mediaUrl, thumbUrl } from "./media-urls"

function withUserAgent(ua: string) {
  vi.stubGlobal("navigator", { userAgent: ua })
}

afterEach(() => vi.unstubAllGlobals())

describe("media-urls", () => {
  it("T-ui-media-01 [Happy] builds the WebView2 http://<scheme>.localhost form on Windows", () => {
    withUserAgent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/130")
    expect(thumbUrl(12, 320)).toBe("http://mamthumb.localhost/12/320")
    expect(thumbUrl(12, 1024)).toBe("http://mamthumb.localhost/12/1024")
    expect(thumbUrl(12, "strip")).toBe("http://mamthumb.localhost/12/strip")
    expect(mediaUrl(12)).toBe("http://mammedia.localhost/12")
    expect([THUMB_SCHEME, MEDIA_SCHEME, STRIP_FRAMES]).toEqual(["mamthumb", "mammedia", 5])
  })

  it("T-ui-media-02 [Happy] uses <scheme>://localhost elsewhere and appends the revision as a cache-busting query", () => {
    withUserAgent("Mozilla/5.0 (X11; Linux x86_64)")
    expect(thumbUrl(7, 320, 3)).toBe("mamthumb://localhost/7/320?v=3")
    expect(mediaUrl(7)).toBe("mammedia://localhost/7")
  })

  it("T-ui-media-03 [Validation] rejects ids the server would answer with 400", () => {
    withUserAgent("Windows")
    for (const bad of [0, -1, 1.5, Number.NaN, Number.POSITIVE_INFINITY, 2 ** 53]) {
      expect(() => thumbUrl(bad, 320)).toThrow(RangeError)
      expect(() => mediaUrl(bad)).toThrow(RangeError)
    }
  })
})
