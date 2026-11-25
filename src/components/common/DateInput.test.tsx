/**
 * DateInput — 분류: Happy(직접 입력·달력 선택·지우기·오늘) / Validation(형식 판정) / Business rule(표시 언어에 따른 달력 문구)
 */
import { act, fireEvent, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"
import i18n from "@/i18n"
import { DateInput } from "./DateInput"
import { calendarLocale, formatDateText, parseDateText } from "./dateText"

const EN = { dateInput: { placeholder: "yyyy-mm-dd", openCalendar: "Open calendar", clear: "Clear", today: "Today" } }

beforeAll(() => {
  i18n.addResourceBundle("en", "translation", EN, true, true)
})

afterEach(async () => {
  vi.useRealTimers()
  await act(() => i18n.changeLanguage("ko"))
})

function Harness({ initial = "", spy }: { initial?: string; spy?: (v: string, s: string) => void }) {
  const [v, setV] = useState(initial)
  return (
    <DateInput
      aria-label="촬영일"
      value={v}
      onValueChange={(t, s) => {
        setV(t)
        spy?.(t, s)
      }}
    />
  )
}

describe("T-ui-date-parse", () => {
  it("T-ui-date-01 [Validation] parseDateText accepts real dates only", () => {
    expect(formatDateText(parseDateText("2026-11-15")!)).toBe("2026-11-15")
    for (const bad of ["", "2026-1-5", "2026-02-30", "2026-13-01", "11/15/2026", "2026-11-15x"]) {
      expect(parseDateText(bad)).toBeUndefined()
    }
  })

  it("T-ui-date-02 [Boundary] calendarLocale maps supported languages and falls back to en-US", () => {
    expect(calendarLocale("ko").code).toBe("ko")
    expect(calendarLocale("de-AT").code).toBe("de")
    expect(calendarLocale("ja").code).toBe("ja")
    expect(calendarLocale("zh").code).toBe("en-US")
    expect(calendarLocale(undefined).code).toBe("en-US")
  })
})

describe("T-ui-date-input", () => {
  it("T-ui-date-03 [Happy] typing emits the text as typed; placeholder comes from the language", async () => {
    const spy = vi.fn()
    render(<Harness spy={spy} />)
    const box = screen.getByLabelText("촬영일")
    expect(box).toHaveAttribute("placeholder", "년-월-일")
    fireEvent.change(box, { target: { value: "2026-11-15" } })
    expect(spy).toHaveBeenLastCalledWith("2026-11-15", "typed")
    expect(box).toHaveValue("2026-11-15")
  })

  it("T-ui-date-04 [Happy] picking a day in the calendar emits YYYY-MM-DD and closes the popover", async () => {
    const user = userEvent.setup()
    const spy = vi.fn()
    render(<Harness initial="2026-11-15" spy={spy} />)
    await user.click(screen.getByRole("button", { name: "달력 열기" }))
    const grid = await screen.findByRole("grid")
    await user.click(within(grid).getByRole("button", { name: /20일/ }))
    expect(spy).toHaveBeenLastCalledWith("2026-11-20", "picked")
    expect(screen.getByLabelText("촬영일")).toHaveValue("2026-11-20")
    expect(screen.queryByRole("grid")).not.toBeInTheDocument()
  })

  it("T-ui-date-05 [Happy] Clear empties the value and Today fills the current local date", async () => {
    const user = userEvent.setup()
    vi.useFakeTimers({ toFake: ["Date"], now: new Date(2026, 5, 3, 12) })
    const spy = vi.fn()
    render(<Harness initial="2026-11-15" spy={spy} />)
    await user.click(screen.getByRole("button", { name: "달력 열기" }))
    await user.click(await screen.findByRole("button", { name: "지우기" }))
    expect(spy).toHaveBeenLastCalledWith("", "picked")
    expect(screen.getByLabelText("촬영일")).toHaveValue("")
    await user.click(screen.getByRole("button", { name: "달력 열기" }))
    await user.click(await screen.findByRole("button", { name: "오늘" }))
    expect(spy).toHaveBeenLastCalledWith("2026-06-03", "picked")
  })

  it("T-ui-date-06 [Business rule] regression: English language shows English month/weekday names and no Korean", async () => {
    const user = userEvent.setup()
    await act(() => i18n.changeLanguage("en"))
    render(<Harness initial="2026-11-15" />)
    expect(screen.getByLabelText("촬영일")).toHaveAttribute("placeholder", "yyyy-mm-dd")
    await user.click(screen.getByRole("button", { name: "Open calendar" }))
    const grid = await screen.findByRole("grid")
    expect(grid).toHaveAccessibleName(/November 2026/)
    expect(within(grid).getByRole("button", { name: /^Monday, November 2nd, 2026/ })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Today" })).toBeInTheDocument()
    expect(document.body.textContent ?? "").not.toMatch(/[가-힣]/)
  })

  it("T-ui-date-07 [Business rule] Korean language shows Korean month and weekday names", async () => {
    const user = userEvent.setup()
    render(<Harness initial="2026-11-15" />)
    await user.click(screen.getByRole("button", { name: "달력 열기" }))
    const grid = await screen.findByRole("grid")
    expect(grid).toHaveAccessibleName(/2026년 11월/)
    expect(within(grid).getByRole("button", { name: /2026년 11월 2일 월요일/ })).toBeInTheDocument()
  })

  it("T-ui-date-08 [Validation] aria-invalid / aria-describedby pass through to the text box", () => {
    render(<DateInput aria-label="d" aria-invalid aria-describedby="err" value="x" onValueChange={() => {}} />)
    const box = screen.getByLabelText("d")
    expect(box).toHaveAttribute("aria-invalid", "true")
    expect(box).toHaveAttribute("aria-describedby", "err")
  })
})
