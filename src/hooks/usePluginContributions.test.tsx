// Categories: Happy (T-ui-pmenu-20 refresh on plugin://changed) / External (T-ui-pmenu-21 no event channel)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { describe, expect, it, vi } from "vitest"
import { usePluginContributions, usePluginContributionsSync } from "./usePluginContributions"

const h = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}))
vi.mock("@tauri-apps/api/core", () => ({ invoke: h.invoke }))
vi.mock("@tauri-apps/api/event", () => ({ listen: h.listen }))

const wrap = () => {
  const qc = new QueryClient()
  return ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>
}
const both = () => {
  usePluginContributionsSync()
  return usePluginContributions()
}

describe("plugin contributions cache", () => {
  it("T-ui-pmenu-20 [Happy] is fetched once and refetched when plugin://changed arrives", async () => {
    // Arrange
    let emit: (p: unknown) => void = () => undefined
    h.listen.mockImplementation(async (_e: string, cb: (e: { payload: unknown }) => void) => {
      emit = (payload) => cb({ payload })
      return () => undefined
    })
    h.invoke.mockResolvedValue({ menuItems: [], exportProviders: [] })
    const { result } = renderHook(both, { wrapper: wrap() })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(h.invoke).toHaveBeenCalledTimes(1)
    // Act
    await waitFor(() => expect(h.listen).toHaveBeenCalledWith("plugin://changed", expect.any(Function)))
    emit({ pluginId: "x", reason: "enabled" })
    // Assert
    await waitFor(() => expect(h.invoke).toHaveBeenCalledTimes(2))
  })

  it("T-ui-pmenu-21 [External] a missing event channel does not break the query", async () => {
    // Arrange
    h.invoke.mockReset()
    h.invoke.mockResolvedValue({ menuItems: [], exportProviders: [] })
    h.listen.mockRejectedValue(new Error("no tauri"))
    // Act
    const { result } = renderHook(both, { wrapper: wrap() })
    // Assert
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
  })
})
