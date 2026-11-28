// Categories: Happy (01 02 03 05) / Boundary (04 fallback, 07) / Validation (06) / External (08)
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { openFolderSettings } from "@/test/folderDialog"
import { openSettingsPage } from "@/test/settings"

const PATH = "D:\\Photos"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup(preset?: string) {
  stubLayout()
  const user = userEvent.setup()
  const api = createMockApi({ count: 10 })
  if (preset) await api.setFolderAlias(1, preset)
  renderApp(api, { section: "folders" })
  const nav = await screen.findByRole("navigation", { name: "라이브러리 탐색" })
  return { user, api, nav }
}

async function openDialog(user: ReturnType<typeof userEvent.setup>) {
  const settings = await openSettingsPage(user, "관리 폴더")
  const dialog = await openFolderSettings(user, settings, PATH)
  return { settings, dialog, input: within(dialog).getByRole("textbox", { name: "별칭" }) }
}

describe("managed folder alias", () => {
  it("T-ui-alias-03 [Happy] sidebar shows the alias and exposes the full path as tooltip", async () => {
    const { nav } = await setup("사진")
    const row = await within(nav).findByRole("button", { name: "사진" })
    expect(row).toHaveAttribute("title", PATH)
    expect(within(nav).queryByRole("button", { name: PATH })).not.toBeInTheDocument()
  })

  it("T-ui-alias-04 [Boundary] without alias the sidebar shows the path and no tooltip", async () => {
    const { nav } = await setup()
    const row = await within(nav).findByRole("button", { name: PATH })
    expect(row).not.toHaveAttribute("title")
  })

  it("T-ui-alias-05 [Happy] saving an alias updates the settings row (alias + path) and the sidebar; Enter submits", async () => {
    const { user, api, nav } = await setup()
    const spy = vi.spyOn(api, "setFolderAlias")
    const { settings, input } = await openDialog(user)
    expect(input).toHaveValue("")
    await user.type(input, "  내 사진  {Enter}")
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(spy).toHaveBeenCalledWith(1, "내 사진")
    expect(await within(nav).findByRole("button", { name: "내 사진", hidden: true })).toHaveAttribute("title", PATH)
    expect(within(settings).getByText("내 사진")).toBeInTheDocument()
    expect(within(settings).getByText("내 사진")).toHaveAttribute("title", PATH) // 전체 경로는 툴팁
  })

  it("T-ui-alias-06 [Happy] clearing the input removes the alias and the dialog is prefilled", async () => {
    const { user, api, nav } = await setup("사진")
    const spy = vi.spyOn(api, "setFolderAlias")
    const { input, dialog } = await openDialog(user)
    expect(input).toHaveValue("사진")
    await user.clear(input)
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(spy).toHaveBeenCalledWith(1, null)
    expect(await within(nav).findByRole("button", { name: PATH, hidden: true })).not.toHaveAttribute("title")
  })

  it("T-ui-alias-07 [Validation] more than 64 characters shows an inline error and makes no API call", async () => {
    const { user, api } = await setup()
    const spy = vi.spyOn(api, "setFolderAlias")
    const { input, dialog } = await openDialog(user)
    await user.click(input)
    await user.paste("가".repeat(65))
    const alert = await within(dialog).findByRole("alert")
    expect(alert).toHaveTextContent("64자 이하")
    expect(input).toHaveAttribute("aria-invalid", "true")
    expect(input.getAttribute("aria-describedby")).toBe(alert.id)
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    expect(spy).not.toHaveBeenCalled()
    expect(screen.getByRole("dialog", { name: "관리 폴더 설정" })).toBeInTheDocument()
  })

  it("T-ui-alias-08 [External] API rejection shows an inline alert and keeps the dialog open", async () => {
    const { user, api } = await setup()
    vi.spyOn(api, "setFolderAlias").mockRejectedValue({ code: "not_found", message: "x" })
    const { input, dialog } = await openDialog(user)
    await user.type(input, "사진")
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("대상을 찾을 수 없습니다.")
    expect(input).toHaveAttribute("aria-invalid", "true")
  })

  it("T-ui-alias-09 [Business rule] mock backend rejects >64 chars and unknown ids like the Rust command", async () => {
    const api = createMockApi({ count: 3 })
    await expect(api.setFolderAlias(1, "a".repeat(65))).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.setFolderAlias(999, "a")).rejects.toMatchObject({ code: "not_found" })
    await api.setFolderAlias(1, "   ")
    expect((await api.listFolders())[0].alias).toBeNull()
  })
})
