// Categories: Happy / External (Rust-shaped payloads, command names, args)
import { beforeEach, describe, expect, it, vi } from "vitest"
import { invoke } from "@tauri-apps/api/core"
import { listLocales, openLocaleFolder, readLocale } from "./ipc"
import type { LocaleList } from "./ipc-contract"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

beforeEach(() => {
  vi.mocked(invoke).mockReset()
})

describe("locale ipc", () => {
  it("T-ui-locale-13 [Happy] listLocales invokes list_locales with no args and returns the Rust shape", async () => {
    // Arrange
    const list: LocaleList = {
      dir: "C:\\Users\\u\\AppData\\Local\\MAMDesktop\\locales",
      locales: [
        { code: "en", name: "English", error: null },
        { code: "ja", name: null, error: "invalid_json" },
      ],
    }
    vi.mocked(invoke).mockResolvedValue(list)
    // Act + Assert
    expect(await listLocales()).toEqual(list)
    expect(invoke).toHaveBeenCalledWith("list_locales")
  })

  it("T-ui-locale-14 [Happy] readLocale sends { code }; a CommandError rejection surfaces unchanged", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce({ _meta: { name: "English" }, common: { cancel: "Cancel" } })
    // Act + Assert
    expect(await readLocale("en")).toEqual({ _meta: { name: "English" }, common: { cancel: "Cancel" } })
    expect(invoke).toHaveBeenCalledWith("read_locale", { code: "en" })
    const err = { code: "not_found", message: "locale not found" }
    vi.mocked(invoke).mockRejectedValueOnce(err)
    await expect(readLocale("xx")).rejects.toEqual(err)
  })

  it("T-ui-locale-15 [Happy] openLocaleFolder invokes open_locale_folder with no args", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)
    await openLocaleFolder()
    expect(invoke).toHaveBeenCalledWith("open_locale_folder")
  })
})
