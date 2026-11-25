// Categories: Validation (T-ui-aicli-api-01) / Happy (T-ui-aicli-api-02) / Business rule (T-ui-aicli-api-03)
import { describe, expect, it } from "vitest"
import { rejection } from "@/test/ai-helpers"
import { createMockApi } from "./mock"

describe("mock api ai cli (subscription)", () => {
  it("T-ui-aicli-api-01 [Validation] subscription mode is claude-only; cli is exposed only in subscription mode", async () => {
    // Arrange
    const api = createMockApi({ count: 10 })
    // Act + Assert
    expect((await rejection(api.setProviderAuthMode("openai", "subscription"))).code).toBe("invalid_input")
    const before = (await api.listAiProviders()).find((p) => p.provider === "claude")
    expect(before).toMatchObject({ authMode: "apiKey", subscriptionSupported: true, cli: null })
    const after = await api.setProviderAuthMode("claude", "subscription")
    expect(after.cli).toMatchObject({ found: true, supported: true, apiKeyEnv: false, account: { loggedIn: false } })
    expect((await api.listAiProviders()).find((p) => p.provider === "openai")?.subscriptionSupported).toBe(false)
  })

  it("T-ui-aicli-api-02 [Happy] login completes after the configured polls, done is delivered once, then logout; busy rejects with ai_busy", async () => {
    // Arrange
    const api = createMockApi({ count: 10, ai: { cli: { loginPolls: 2 } } })
    await api.setProviderAuthMode("claude", "subscription")
    // Act + Assert
    expect(await api.aiCliLoginStatus("claude")).toEqual({ state: "none" })
    expect(await api.aiCliLoginStart("claude")).toMatchObject({ state: "waiting", url: expect.stringMatching(/^https:\/\//) })
    expect(await api.openAiCliLoginUrl("claude")).toBe(true)
    expect(await api.aiCliLoginStatus("claude")).toMatchObject({ state: "waiting" })
    expect(await api.aiCliLoginStatus("claude")).toEqual({ state: "done", verified: true })
    expect(await api.aiCliLoginStatus("claude")).toEqual({ state: "none" })
    expect((await api.detectAiCli("claude")).cli?.account).toMatchObject({ loggedIn: true })
    expect((await api.aiCliLogout("claude")).cli?.account).toEqual({ loggedIn: false })
    const busy = createMockApi({ count: 10, ai: { cli: { loggedIn: true, busy: true } } })
    await busy.setProviderAuthMode("claude", "subscription")
    expect((await rejection(busy.aiCliLogout("claude"))).code).toBe("ai_busy")
  })

  it("T-ui-aicli-api-03 [Business rule] checkProvider in subscription mode maps cli states to error codes; cancel clears the session", async () => {
    // Arrange
    const missing = createMockApi({ count: 10, ai: { cli: { found: false } } })
    await missing.setProviderAuthMode("claude", "subscription")
    const out = createMockApi({ count: 10 })
    await out.setProviderAuthMode("claude", "subscription")
    // Act + Assert
    expect((await missing.checkProvider("claude")).errorCode).toBe("cli_not_found")
    expect(await missing.aiCliLoginStart("claude")).toEqual({ state: "failed", errorCode: "cli_not_found" })
    expect((await out.checkProvider("claude")).errorCode).toBe("cli_not_logged_in")
    await out.aiCliLoginStart("claude")
    expect(await out.aiCliLoginCancel("claude")).toEqual({ state: "none" })
    expect(await out.openAiCliLoginUrl("claude")).toBe(false)
  })

  it("T-ui-aicli-api-04 [Business rule] switching to subscription sets the default provider only when none is set", async () => {
    // Arrange
    const api = createMockApi({ count: 10 })
    const before = await api.getAiSettings()
    // Act
    await api.setProviderAuthMode("claude", "subscription")
    // Assert
    expect(before.defaultProvider).toBeNull()
    expect((await api.getAiSettings()).defaultProvider).toBe("claude")
    // Act 2: 이미 있으면 유지
    await api.updateAiSettings({ defaultProvider: "openai" })
    await api.setProviderAuthMode("claude", "apiKey")
    await api.setProviderAuthMode("claude", "subscription")
    expect((await api.getAiSettings()).defaultProvider).toBe("openai")
  })
})
