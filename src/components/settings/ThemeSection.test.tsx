// Categories: Happy / Business rule (D1·D5) / Validation (error path) — T-ui-theme-set-01~07
import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import type { Settings } from "@/lib/ipc-contract"

const mutate = vi.fn()
let osDark = false
vi.mock("@/hooks/useSettings", () => ({ useUpdateSettings: () => ({ mutate }) }))
vi.mock("@/theme/useOsDark", () => ({ useOsDark: () => osDark }))

import { ThemeSection } from "./ThemeSection"

function renderSection(theme: Settings["theme"]) {
  return render(<ThemeSection settings={{ theme } as Settings} />)
}

beforeEach(() => {
  mutate.mockReset()
  osDark = false
})

describe("ThemeSection", () => {
  it("T-ui-theme-set-01 [Happy] 5개 옵션을 radiogroup 으로 렌더하고 현재 선택을 체크한다", () => {
    renderSection({ mode: "dim", systemDark: "dark" })
    expect(screen.getByRole("radiogroup", { name: "화면 테마" })).toBeInTheDocument()
    expect(screen.getAllByRole("radio", { name: /^(시스템|라이트|라이트 그레이|딤|다크)$/ })).toHaveLength(5)
    expect(screen.getByRole("radio", { name: "딤" })).toBeChecked()
    expect(screen.getByRole("radio", { name: "라이트" })).not.toBeChecked()
  })

  it("T-ui-theme-set-02 [Happy] 딤 클릭 시 현재 systemDark 와 함께 저장한다", async () => {
    renderSection({ mode: "light", systemDark: "dark" })
    await userEvent.setup().click(screen.getByRole("radio", { name: "딤" }))
    expect(mutate).toHaveBeenCalledWith({ theme: { mode: "dim", systemDark: "dark" } }, expect.anything())
  })

  it("T-ui-theme-set-08 [Happy] 라이트 그레이 클릭 시 mode gray 와 현재 systemDark 를 저장한다", async () => {
    renderSection({ mode: "light", systemDark: "dark" })
    await userEvent.setup().click(screen.getByRole("radio", { name: "라이트 그레이" }))
    expect(mutate).toHaveBeenCalledWith({ theme: { mode: "gray", systemDark: "dark" } }, expect.anything())
  })

  it("T-ui-theme-set-03[Business] 보조 선택지는 시스템일 때만 보인다", () => {
    const { unmount } = renderSection({ mode: "system", systemDark: "dim" })
    expect(screen.getByRole("radiogroup", { name: "Windows 가 다크일 때" })).toBeInTheDocument()
    unmount()
    renderSection({ mode: "light", systemDark: "dim" })
    expect(screen.queryByRole("radiogroup", { name: "Windows 가 다크일 때" })).not.toBeInTheDocument()
  })

  it("T-ui-theme-set-04 [Happy] 보조 선택지에서 다크 선택 시 mode 는 system 유지", async () => {
    renderSection({ mode: "system", systemDark: "dim" })
    expect(screen.getByRole("radio", { name: "딤 (기본)" })).toBeChecked()
    const sub = screen.getByRole("radiogroup", { name: "Windows 가 다크일 때" })
    await userEvent.setup().click(within(sub).getByRole("radio", { name: "다크" }))
    expect(mutate).toHaveBeenCalledWith({ theme: { mode: "system", systemDark: "dark" } }, expect.anything())
  })

  it("T-ui-theme-set-05 [Business] 시스템을 벗어났다 돌아와도 systemDark 가 유지된다", async () => {
    const user = userEvent.setup()
    const { unmount } = renderSection({ mode: "system", systemDark: "dark" })
    await user.click(screen.getByRole("radio", { name: "라이트" }))
    expect(mutate).toHaveBeenLastCalledWith({ theme: { mode: "light", systemDark: "dark" } }, expect.anything())
    unmount()
    renderSection({ mode: "light", systemDark: "dark" })
    await user.click(screen.getByRole("radio", { name: "시스템" }))
    expect(mutate).toHaveBeenLastCalledWith({ theme: { mode: "system", systemDark: "dark" } }, expect.anything())
  })

  it("T-ui-theme-set-06 [External] 현재 Windows 앱 모드 상태 줄이 OS 다크/라이트를 반영한다", () => {
    osDark = true
    const { unmount } = renderSection({ mode: "system", systemDark: "dim" })
    expect(screen.getByRole("status")).toHaveTextContent("현재 Windows 앱 모드: 다크 · “딤” 적용 중")
    unmount()
    osDark = false
    renderSection({ mode: "system", systemDark: "dim" })
    expect(screen.getByRole("status")).toHaveTextContent("현재 Windows 앱 모드: 라이트 · “라이트” 적용 중")
  })

  it("T-ui-theme-set-07 [Validation] 저장 실패 시 인라인 오류를 표시한다", async () => {
    mutate.mockImplementation((_p, opts) => opts.onError(new Error("boom")))
    renderSection({ mode: "light", systemDark: "dim" })
    await userEvent.setup().click(screen.getByRole("radio", { name: "다크" }))
    expect(await screen.findByRole("alert")).toBeInTheDocument()
  })
})
