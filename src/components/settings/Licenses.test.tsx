// Categories: Happy (T-ui-lic-01~03, 06) / Boundary + Validation (T-ui-lic-04, 05, 07) / Business rule (T-ui-lic-08~10)
import { existsSync, readFileSync } from "node:fs"
import { resolve } from "node:path"
import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { MarkdownDocument } from "@/components/markdown/MarkdownDocument"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"
import { LicensesTab } from "./LicensesTab"
import { parseLicenses } from "./licensesParse"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const LICENSES = [
  "# Third-Party Licenses",
  "<!-- generated-by: scripts/dump-licenses.mjs; cargo-lock-sha256: aa; pnpm-lock-sha256: bb; packages: 3 -->",
  "Intro line.",
  "",
  "## MIT License (MIT) — 2 packages",
  "### Packages",
  "- `alpha` 1.0.0 (rust) — Copyright (c) A",
  "- `beta` 2.0.0 (npm) — Copyright (c) B",
  "### License text",
  "```text",
  "MIT-LICENSE-BODY",
  "## not a section (inside fence)",
  "```",
  "",
  "## MIT License (MIT, variant 2) — 1 packages",
  "### Packages",
  "- `gamma` 3.0.0 (rust) — Copyright holders: see authors: X",
  "### License text",
  "```text",
  "VARIANT-BODY",
  "```",
  "### NOTICE",
  "#### `gamma` 3.0.0",
  "```",
  "GAMMA-NOTICE",
  "```",
].join("\n")

describe("parseLicenses", () => {
  it("T-ui-lic-01 [Happy] splits contract sections by '## ', ignores '## ' inside code fences, reads package count", () => {
    // Act
    const r = parseLicenses(LICENSES)
    // Assert
    expect(r.packageCount).toBe(3)
    expect(r.sections.map((s) => s.title)).toEqual(["MIT License (MIT) — 2 packages", "MIT License (MIT, variant 2) — 1 packages"])
    expect(r.sections[0].body).toContain("## not a section (inside fence)")
    expect(r.sections[1].body).toContain("GAMMA-NOTICE")
  })

  it("T-ui-lic-02 [Boundary] a section without NOTICE keeps only its own body; CRLF input parses the same", () => {
    // Act
    const r = parseLicenses(LICENSES.replace(/\n/g, "\r\n"))
    // Assert
    expect(r.sections).toHaveLength(2)
    expect(r.sections[0].body).not.toContain("NOTICE")
    expect(r.sections[0].body).not.toContain("\r")
  })

  it.each([["empty", ""], ["no sections", "just text\nno headings"], ["bad meta", "<!-- packages: x -->\n## Only title"]])(
    "T-ui-lic-03 [Validation] malformed input (%s) does not throw",
    (_name, input) => {
      // Act + Assert
      expect(() => parseLicenses(input)).not.toThrow()
      expect(parseLicenses(input).packageCount).toBeNull()
    },
  )
})

describe("LicensesTab", () => {
  it("T-ui-lic-04 [Happy] notices tab renders the document with real headings", () => {
    // Arrange + Act
    render(<LicensesTab notices={"# Third-Party Notices\n\n## 1. Bundled\n\n| a | b |\n|---|---|\n| 1 | 2 |"} licenses={LICENSES} />)
    // Assert
    expect(screen.getByRole("heading", { level: 1, name: "Third-Party Notices" })).toBeInTheDocument()
    expect(screen.getByRole("heading", { level: 2, name: "1. Bundled" })).toBeInTheDocument()
    expect(screen.getByRole("table")).toBeInTheDocument()
  })

  it("T-ui-lic-05 [Business rule] license sections are collapsed by default; text renders only after expanding and is removed on collapse", async () => {
    // Arrange
    const user = userEvent.setup()
    render(<LicensesTab notices="x" licenses={LICENSES} />)
    await user.click(screen.getByRole("tab", { name: "전체 라이선스" }))
    expect(screen.getByText("총 3개 패키지")).toBeInTheDocument()
    const trigger = screen.getByRole("button", { name: /^MIT License \(MIT\) — 2 packages/ })
    // Assert: 접힘
    expect(trigger).toHaveAttribute("aria-expanded", "false")
    expect(screen.queryByText(/MIT-LICENSE-BODY/)).not.toBeInTheDocument()
    expect(screen.queryByText(/VARIANT-BODY/)).not.toBeInTheDocument()
    // Act: 펼침
    await user.click(trigger)
    // Assert: 펼친 절만 렌더
    expect(screen.getByText(/MIT-LICENSE-BODY/)).toBeInTheDocument()
    expect(screen.queryByText(/VARIANT-BODY/)).not.toBeInTheDocument()
    // Act: 다시 접기
    await user.click(trigger)
    expect(screen.queryByText(/MIT-LICENSE-BODY/)).not.toBeInTheDocument()
  })

  it("T-ui-lic-06 [Boundary] an empty / unparsable licenses document shows an empty state instead of crashing", async () => {
    // Arrange
    const user = userEvent.setup()
    render(<LicensesTab notices="" licenses="garbage" />)
    // Assert: 고지 탭
    expect(screen.getByText("표시할 문서가 없습니다")).toBeInTheDocument()
    // 전체 라이선스 탭
    await user.click(screen.getByRole("tab", { name: "전체 라이선스" }))
    expect(screen.getByText("표시할 문서가 없습니다")).toBeInTheDocument()
  })
})

describe("MarkdownDocument safety", () => {
  it("T-ui-lic-07 [Validation] raw HTML (<script>, <img onerror>) is not rendered as elements", () => {
    // Arrange + Act
    const { container } = render(
      <MarkdownDocument source={'before <script>window.__pwned = 1</script> <img src="x" onerror="window.__pwned = 2"> after\n\n<div onclick="x()">raw</div>'} />,
    )
    // Assert
    expect(container.querySelector("script")).toBeNull()
    expect(container.querySelector("img")).toBeNull()
    expect(container.querySelector("[onclick]")).toBeNull()
    expect(container.querySelector("[onerror]")).toBeNull()
  })

  it("T-ui-lic-08 [Business rule] links are not anchors: clicking does not navigate and the copy button writes the URL to the clipboard", async () => {
    // Arrange
    const user = userEvent.setup()
    const writeText = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue(undefined)
    const before = window.location.href
    const { container } = render(<MarkdownDocument source="See [FFmpeg](https://ffmpeg.org/download.html) now" />)
    // Act
    await user.click(screen.getByText("FFmpeg"))
    await user.click(screen.getByRole("button", { name: /https:\/\/ffmpeg\.org\/download\.html/ }))
    // Assert
    expect(container.querySelector("a")).toBeNull()
    expect(window.location.href).toBe(before)
    expect(writeText).toHaveBeenCalledWith("https://ffmpeg.org/download.html")
  })

  it("T-ui-lic-09 [Validation] javascript: links render as plain text without a copy button", () => {
    // Arrange + Act
    const { container } = render(<MarkdownDocument source="[click](javascript:alert(1))" />)
    // Assert
    expect(container.querySelector("a")).toBeNull()
    expect(screen.queryByRole("button")).not.toBeInTheDocument()
    expect(screen.getByText("click")).toBeInTheDocument()
  })
})

describe("settings menu — licenses page", () => {
  it("T-ui-lic-10 [Happy] 오픈소스 라이선스 is the last menu item, reachable by End key and click, with title and English-original description", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50 }))
    const main = await openSettingsPage(user)
    const nav = within(await screen.findByRole("navigation", { name: "설정 메뉴" }))
    const buttons = nav.getAllByRole("button")
    expect(buttons[buttons.length - 1]).toHaveAccessibleName("오픈소스 라이선스")
    // Act: End 키
    nav.getByRole("button", { name: "일반" }).focus()
    await user.keyboard("{End}")
    // Assert
    expect(nav.getByRole("button", { name: "오픈소스 라이선스" })).toHaveAttribute("aria-current", "page")
    expect(within(main).getByRole("heading", { level: 2, name: "오픈소스 라이선스" })).toBeInTheDocument()
    expect(within(main).getByText(/라이선스 원문은 영문입니다/)).toBeInTheDocument()
    expect(within(main).getByRole("tab", { name: "고지" })).toBeInTheDocument()
    // Act: 클릭으로도 진입
    await user.click(nav.getByRole("button", { name: "일반" }))
    await user.click(nav.getByRole("button", { name: "오픈소스 라이선스" }))
    expect(within(main).getByRole("tab", { name: "전체 라이선스" })).toBeInTheDocument()
  })
})

/** 파서와 독립적인 CommonMark 펜스 스캔 — 백틱 펜스만, 닫는 줄은 같거나 더 긴 백틱 런. */
function scanFences(text: string): { headings: number; closed: boolean } {
  let open = 0
  let headings = 0
  for (const line of text.split("\n")) {
    const t = line.replace(/^ {0,3}/, "")
    const run = /^`+/.exec(t)?.[0].length ?? 0
    if (open === 0) {
      if (run >= 3 && !t.slice(run).includes("`")) open = run
      else if (line.startsWith("## ")) headings++
    } else if (run >= open && t.slice(run).trim() === "") {
      open = 0
    }
  }
  return { headings, closed: open === 0 }
}

describe("parseLicenses — fences and the real generated file", () => {
  it("T-ui-lic-11 [Boundary] a 4-backtick fence containing ``` and '## ' lines is not split or closed early", () => {
    // Arrange
    const src = ["## A (X) — 1 packages", "````text", "```", "## inside", "```", "# also inside", "````", "tail", "## B (Y) — 1 packages", "body"].join("\n")
    // Act
    const r = parseLicenses(src)
    // Assert
    expect(r.sections.map((s) => s.title)).toEqual(["A (X) — 1 packages", "B (Y) — 1 packages"])
    expect(r.sections[0].body).toContain("## inside")
    expect(r.sections[0].body).toContain("tail")
  })

  it("T-ui-lic-12 [Happy] the real THIRD_PARTY_LICENSES.md: section count equals fence-aware '## ' count and no section has an unclosed fence", () => {
    // Arrange
    const file = resolve(process.cwd(), "THIRD_PARTY_LICENSES.md")
    if (!existsSync(file)) return // 생성 전 체크아웃 — 건너뜀
    const text = readFileSync(file, "utf8").replace(/\r\n/g, "\n")
    // Act
    const t0 = performance.now()
    const parsed = parseLicenses(text)
    const parseMs = performance.now() - t0
    // Assert
    expect(parsed.sections.length).toBe(scanFences(text).headings)
    expect(parsed.sections.length).toBeGreaterThan(50)
    expect(parsed.packageCount).toBeGreaterThan(0)
    for (const s of parsed.sections) expect(scanFences(s.body).closed, s.title).toBe(true)
    console.info(`[perf] parse real file: ${parseMs.toFixed(1)} ms, ${text.length} chars, ${parsed.sections.length} sections`)
  })

  it("T-ui-lic-13 [Happy] full-licenses tab with the real file renders only collapsed titles (first render measured)", async () => {
    // Arrange
    const file = resolve(process.cwd(), "THIRD_PARTY_LICENSES.md")
    if (!existsSync(file)) return
    const text = readFileSync(file, "utf8").replace(/\r\n/g, "\n")
    const user = userEvent.setup()
    render(<LicensesTab notices="x" licenses={text} />)
    // Act
    const t0 = performance.now()
    await user.click(screen.getByRole("tab", { name: "전체 라이선스" }))
    const triggers = screen.getAllByRole("button", { expanded: false })
    const renderMs = performance.now() - t0
    // Assert
    expect(triggers.length).toBe(parseLicenses(text).sections.length)
    expect(screen.queryAllByRole("heading").length).toBe(0)
    console.info(`[perf] full-licenses tab first render (jsdom): ${renderMs.toFixed(0)} ms, ${triggers.length} collapsed sections`)
    // 첫 절 하나를 펼칠 때의 비용
    const t1 = performance.now()
    await user.click(triggers[0])
    console.info(`[perf] expand first section (jsdom): ${(performance.now() - t1).toFixed(0)} ms`)
    expect(triggers[0]).toHaveAttribute("aria-expanded", "true")
  })
})
