// Categories: Happy (T-ui-gensub-06) / Business rule (T-ui-gensub-06 groups, T-ui-gensub-07 CC pressed state, T-ui-gensub-08 no tracks at all)
import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { SEGMENTS } from "@/test/generatedSubtitle"
import { subTrack } from "@/test/subtitles"
import { SubtitleMenu } from "./SubtitleMenu"
import type { GeneratedSubtitleUi } from "./useGeneratedSubtitle"
import type { SubtitleUi } from "./useSubtitles"

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => vi.restoreAllMocks())

const gen = (over: Partial<GeneratedSubtitleUi> = {}): GeneratedSubtitleUi => ({
  segments: SEGMENTS,
  language: "ko",
  available: true,
  visible: true,
  shown: true,
  toggle: vi.fn(),
  ...over,
})
const file = (over: Partial<SubtitleUi> = {}): SubtitleUi => ({ tracks: [subTrack("a.srt", "en")], selectedId: null, effectiveId: null, select: vi.fn(), loading: false, ...over })
const cc = () => screen.getByRole("button", { name: "자막" })

describe("SubtitleMenu — 자막 파일 / 생성된 자막 groups", () => {
  it("T-ui-gensub-06 [Happy] both exist → two labeled groups: file radios (끄기 + tracks) and one generated checkbox using the transcript language", async () => {
    // Arrange
    render(<SubtitleMenu ui={file({ generated: gen({ language: "en" }) })} />)
    // Act
    await userEvent.setup().click(cc())
    const menu = await screen.findByRole("menu")
    // Assert
    expect(within(menu).getByText("자막 파일")).toBeInTheDocument()
    expect(within(menu).getByText("생성된 자막")).toBeInTheDocument()
    expect(within(menu).getAllByRole("menuitemradio").map((el) => el.textContent)).toEqual(["자막 끄기", "영어 (SRT)"])
    const box = within(menu).getByRole("menuitemcheckbox", { name: "영어 (자동 생성)" })
    expect(box).toBeChecked()
  })

  it("T-ui-gensub-06 [Business rule] no file tracks → disabled 자막 파일 없음 (no radios); no generated subtitle → disabled 생성된 자막 없음", async () => {
    // Arrange: 생성된 자막만
    const user = userEvent.setup()
    const a = render(<SubtitleMenu ui={file({ tracks: [], generated: gen() })} />)
    // Act
    await user.click(cc())
    let menu = await screen.findByRole("menu")
    // Assert
    expect(within(menu).getByRole("menuitem", { name: "자막 파일 없음" })).toHaveAttribute("aria-disabled", "true")
    expect(within(menu).queryByRole("menuitemradio")).not.toBeInTheDocument()
    expect(within(menu).getByRole("menuitemcheckbox", { name: "한국어 (자동 생성)" })).toBeInTheDocument()
    a.unmount()
    // Arrange: 자막 파일만
    render(<SubtitleMenu ui={file({ generated: gen({ available: false, shown: false, segments: [], language: null }) })} />)
    // Act
    await user.click(cc())
    menu = await screen.findByRole("menu")
    // Assert
    expect(within(menu).getByRole("menuitem", { name: "생성된 자막 없음" })).toHaveAttribute("aria-disabled", "true")
    expect(within(menu).queryByRole("menuitemcheckbox")).not.toBeInTheDocument()
    expect(within(menu).getAllByRole("menuitemradio")).toHaveLength(2)
  })

  it("T-ui-gensub-06 [Happy] choosing the checkbox calls toggle once; the file radio behavior is unchanged (select(null) for 끄기)", async () => {
    // Arrange
    const user = userEvent.setup()
    const g = gen()
    const ui = file({ generated: g, effectiveId: "a.srt" })
    render(<SubtitleMenu ui={ui} />)
    // Act
    await user.click(cc())
    await user.click(await screen.findByRole("menuitemcheckbox", { name: "한국어 (자동 생성)" }))
    // Assert
    expect(g.toggle).toHaveBeenCalledTimes(1)
    expect(ui.select).not.toHaveBeenCalled()
    // Act: 메뉴는 열린 채 — 끄기
    await user.click(await screen.findByRole("menuitemradio", { name: "자막 끄기" }))
    // Assert
    expect(ui.select).toHaveBeenCalledWith(null)
  })

  it("T-ui-gensub-07 [Business rule] CC aria-pressed is true when either is showing, false when neither", () => {
    // Arrange + Act + Assert: 파일만
    const a = render(<SubtitleMenu ui={file({ effectiveId: "a.srt", generated: gen({ shown: false, visible: false }) })} />)
    expect(cc()).toHaveAttribute("aria-pressed", "true")
    a.unmount()
    // 생성된 자막만
    const b = render(<SubtitleMenu ui={file({ generated: gen() })} />)
    expect(cc()).toHaveAttribute("aria-pressed", "true")
    b.unmount()
    // 둘 다 꺼짐 (자막은 있지만 표시 안 함)
    render(<SubtitleMenu ui={file({ generated: gen({ shown: false, visible: false }) })} />)
    expect(cc()).toHaveAttribute("aria-pressed", "false")
    expect(cc()).toBeEnabled()
  })

  it("T-ui-gensub-08 [Business rule] no file tracks and no generated subtitle → disabled CC (tooltip text is covered by T-ui-sub-02); a generated subtitle alone enables it", () => {
    // Arrange
    const a = render(<SubtitleMenu ui={file({ tracks: [], generated: gen({ available: false, shown: false, segments: [], language: null }) })} />)
    // Assert
    expect(cc()).toBeDisabled()
    expect(cc()).toHaveAttribute("aria-pressed", "false")
    a.unmount()
    // generated prop 자체가 없어도(기존 호출부) 같다
    const b = render(<SubtitleMenu ui={file({ tracks: [] })} />)
    expect(cc()).toBeDisabled()
    b.unmount()
    render(<SubtitleMenu ui={file({ tracks: [], generated: gen() })} />)
    expect(cc()).toBeEnabled()
  })
})
