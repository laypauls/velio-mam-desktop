// Categories: Happy (T-ui-aicli-01, -05) / Business rule (T-ui-aicli-02, -04, -08) / Validation (T-ui-aicli-03, -07) / External (T-ui-aicli-06) / Concurrency (T-ui-aicli-05, -10)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { useProviders } from "@/hooks/useAi"
import { ApiProvider, type UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { MockAiOptions } from "@/lib/api/mock-ctx"
import { ProviderRow } from "./ProviderRow"
import { LOGIN_POLL_MS } from "./SubscriptionBlock"
import { UsageSection } from "./UsageSection"

afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

function Harness() {
  const providers = useProviders()
  return (
    <ul aria-label="공급자 목록">
      {(providers.data ?? []).map((p) => (
        <ProviderRow key={p.provider} status={p} persist />
      ))}
    </ul>
  )
}

function mount(api: UiApi) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
  return render(
    <QueryClientProvider client={queryClient}>
      <ApiProvider api={api}>
        <Harness />
      </ApiProvider>
    </QueryClientProvider>,
  )
}

/** claude 를 구독 모드로 바꿔 둔 mock api. */
async function subscriptionApi(cli: NonNullable<MockAiOptions["cli"]> = {}) {
  const api = createMockApi({ count: 10, ai: { cli } })
  await api.setProviderAuthMode("claude", "subscription")
  return api
}

async function providerRows() {
  const list = await screen.findByRole("list", { name: "공급자 목록" })
  await waitFor(() => expect(within(list).getAllByRole("listitem")).toHaveLength(3))
  return within(list).getAllByRole("listitem").map((li) => within(li))
}

const claudeRow = async () => (await providerRows())[2]

/** setInterval 만 가짜로 — Promise/setTimeout 은 실제라 waitFor·userEvent 가 그대로 동작한다. */
const fakeIntervals = () => vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] })
const tick = () => act(() => vi.advanceTimersByTimeAsync(LOGIN_POLL_MS))

const WAITING = "브라우저에서 계정 승인을 완료하세요…"
const LOGIN_BTN = "Claude 브라우저로 로그인"
const RECHECK_BTN = "Claude 구독 상태 다시 확인"

describe("subscription auth block (Claude)", () => {
  it("T-ui-aicli-01 [Happy] switching the radio calls setProviderAuthMode and swaps the api-key form for the subscription block, and back", async () => {
    // Arrange
    const api = createMockApi({ count: 10 })
    const spy = vi.spyOn(api, "setProviderAuthMode")
    const user = userEvent.setup()
    mount(api)
    const row = await claudeRow()
    expect(row.getByLabelText("Claude API 키")).toBeInTheDocument()

    // Act
    await user.click(row.getByRole("radio", { name: "Claude 구독 계정 (Claude Code 사용)" }))

    // Assert
    expect(spy).toHaveBeenCalledWith("claude", "subscription")
    expect(await row.findByRole("group", { name: "구독 계정" })).toBeInTheDocument()
    expect(row.queryByLabelText("Claude API 키")).not.toBeInTheDocument()
    expect(row.getByText("로그인이 필요합니다.")).toBeInTheDocument()

    // Act 2: API 키로 복귀
    await user.click(row.getByRole("radio", { name: "API 키" }))
    // Assert 2
    expect(spy).toHaveBeenLastCalledWith("claude", "apiKey")
    expect(await row.findByLabelText("Claude API 키")).toBeInTheDocument()
  })

  it("T-ui-aicli-02 [Business rule] providers without subscription support show no auth-mode radio and keep the api-key ui", async () => {
    // Arrange + Act
    mount(createMockApi({ count: 10 }))
    const [openai, gemini, claude] = await providerRows()
    // Assert
    for (const row of [openai, gemini]) {
      expect(row.queryByRole("radiogroup")).not.toBeInTheDocument()
      expect(row.queryByRole("radio")).not.toBeInTheDocument()
    }
    expect(openai.getByLabelText("OpenAI API 키")).toBeInTheDocument()
    expect(claude.getAllByRole("radio")).toHaveLength(2)
  })

  it("T-ui-aicli-03 [Validation] cli states: not found (install guidance), unsupported version, account unknown, not logged in", async () => {
    // Not found
    const first = mount(await subscriptionApi({ found: false }))
    let row = await claudeRow()
    expect(await row.findByText("컴퓨터에서 Claude Code 를 찾을 수 없습니다.")).toBeInTheDocument()
    expect(row.getByText(/직접 설치한 뒤/)).toBeInTheDocument()
    expect(row.getByRole("button", { name: RECHECK_BTN })).toBeInTheDocument()
    expect(row.queryByRole("button", { name: LOGIN_BTN })).not.toBeInTheDocument()
    first.unmount()

    // Unsupported
    const second = mount(await subscriptionApi({ supported: false }))
    row = await claudeRow()
    expect(await row.findByText(/지원하지 않는 버전/)).toBeInTheDocument()
    expect(row.queryByRole("button", { name: LOGIN_BTN })).not.toBeInTheDocument()
    second.unmount()

    // Account unknown (null) — 로그아웃으로 단정하지 않고 로그인/다시 확인 모두 제공
    const third = mount(await subscriptionApi({ accountUnknown: true }))
    row = await claudeRow()
    expect(await row.findByText(/로그인 상태를 확인하지 못했습니다/)).toBeInTheDocument()
    expect(row.queryByText("로그인이 필요합니다.")).not.toBeInTheDocument()
    expect(row.getByRole("button", { name: RECHECK_BTN })).toBeInTheDocument()
    third.unmount()

    // Not logged in
    mount(await subscriptionApi())
    row = await claudeRow()
    expect(await row.findByText("로그인이 필요합니다.")).toBeInTheDocument()
    expect(row.getByRole("button", { name: LOGIN_BTN })).toBeInTheDocument()
  })

  it("T-ui-aicli-04 [Business rule] logged-in state shows email · plan with logout/recheck, the limits note, and the api-key env warning only when apiKeyEnv", async () => {
    // Arrange + Act
    const view = mount(await subscriptionApi({ loggedIn: true }))
    const row = await claudeRow()
    // Assert
    expect(await row.findByText("user@example.com · max")).toBeInTheDocument()
    expect(row.getByText("로그인됨")).toBeInTheDocument()
    expect(row.getByRole("button", { name: "Claude 구독 로그아웃" })).toBeInTheDocument()
    expect(row.getByRole("button", { name: RECHECK_BTN })).toBeInTheDocument()
    expect(row.getByRole("button", { name: "Claude 연결 확인" })).toBeInTheDocument()
    expect(row.getByText(/백그라운드 일괄 분석이 실행되지 않습니다/)).toBeInTheDocument()
    expect(row.queryByText(/환경변수의 API 키로 과금/)).not.toBeInTheDocument()
    view.unmount()

    // Gate ON: apiKeyEnv
    mount(await subscriptionApi({ loggedIn: true, apiKeyEnv: true }))
    const warned = await claudeRow()
    expect(await warned.findByText(/이 PC 환경변수의 API 키로 과금됩니다/)).toBeInTheDocument()
  })

  it("T-ui-aicli-05 [Happy] login start shows waiting + url + open, polls only while waiting, and flips to logged-in on done", async () => {
    // Arrange
    const api = await subscriptionApi({ loginPolls: 2 })
    const start = vi.spyOn(api, "aiCliLoginStart")
    const open = vi.spyOn(api, "openAiCliLoginUrl")
    const status = vi.spyOn(api, "aiCliLoginStatus")
    const user = userEvent.setup()
    fakeIntervals() // 폴링 interval 이 만들어지기 전에 가짜로 전환
    mount(api)
    const row = await claudeRow()
    await user.click(await row.findByRole("button", { name: LOGIN_BTN }))
    expect(start).toHaveBeenCalledWith("claude")
    expect(await row.findByText(WAITING)).toBeInTheDocument()
    expect(row.getByText(/claude auth login/)).toBeInTheDocument()
    expect(row.getByText(/https:\/\/claude\.com\/cai\/oauth\/authorize/)).toBeInTheDocument()
    await user.click(row.getByRole("button", { name: "Claude 로그인 주소 열기" }))
    expect(open).toHaveBeenCalledWith("claude")
    status.mockClear()

    // Act: 가짜 interval 로 폴링을 결정적으로 진행
    await tick()
    // Assert: 첫 조회 — 아직 진행 중
    expect(status).toHaveBeenCalledTimes(1)
    expect(row.getByText(WAITING)).toBeInTheDocument()
    // Act 2
    await tick()
    // Assert 2: 완료 → 공급자 재조회로 로그인됨 표시, 폴링 중단
    expect(await row.findByText("user@example.com · max")).toBeInTheDocument()
    expect(row.queryByText(WAITING)).not.toBeInTheDocument()
    const calls = status.mock.calls.length
    await tick()
    await tick()
    expect(status.mock.calls.length).toBe(calls)
  })

  it("T-ui-aicli-06 [External] cancel calls aiCliLoginCancel, returns to the login button, and stops polling; unmount also stops polling", async () => {
    // Arrange
    const api = await subscriptionApi({ loginPolls: 5 })
    const cancel = vi.spyOn(api, "aiCliLoginCancel")
    const status = vi.spyOn(api, "aiCliLoginStatus")
    const user = userEvent.setup()
    fakeIntervals()
    const view = mount(api)
    const row = await claudeRow()
    await user.click(await row.findByRole("button", { name: LOGIN_BTN }))
    await row.findByText(WAITING)

    // Act
    await user.click(row.getByRole("button", { name: "Claude 로그인 취소" }))
    // Assert
    expect(cancel).toHaveBeenCalledWith("claude")
    expect(await row.findByRole("button", { name: LOGIN_BTN })).toBeInTheDocument()
    status.mockClear()
    await tick()
    await tick()
    expect(status).not.toHaveBeenCalled()

    // 다시 시작 후 언마운트 → 폴링 중단 (clearInterval)
    await user.click(row.getByRole("button", { name: LOGIN_BTN }))
    await row.findByText(WAITING)
    status.mockClear()
    view.unmount()
    await tick()
    expect(status).not.toHaveBeenCalled()
  })

  it("T-ui-aicli-07 [Validation] logout rejected with ai_busy and a failed login start show inline alerts without leaving the state", async () => {
    // Arrange: 로그아웃 거부
    const user = userEvent.setup()
    const view = mount(await subscriptionApi({ loggedIn: true, busy: true }))
    let row = await claudeRow()
    // Act
    await user.click(await row.findByRole("button", { name: "Claude 구독 로그아웃" }))
    // Assert
    expect(await row.findByRole("alert")).toHaveTextContent("진행 중인 AI 분석이 있어 지금은 처리할 수 없습니다.")
    expect(row.getByText("user@example.com · max")).toBeInTheDocument()
    view.unmount()

    // Arrange 2: 로그인 URL 미출력
    const api = await subscriptionApi()
    vi.spyOn(api, "aiCliLoginStart").mockResolvedValue({ state: "failed", errorCode: "login_no_url" })
    mount(api)
    row = await claudeRow()
    // Act 2
    await user.click(await row.findByRole("button", { name: LOGIN_BTN }))
    // Assert 2
    expect(await row.findByRole("alert")).toHaveTextContent("로그인 주소를 받지 못했습니다")
    expect(row.getByRole("button", { name: LOGIN_BTN })).toBeInTheDocument()
  })

  it("T-ui-aicli-09 [External] a one-shot done/failed result returned by the mount-time status call is handled like polling", async () => {
    // Arrange: 마운트 전에 로그인 세션이 시작돼 있고, 첫 조회에서 done 이 한 번만 전달된다
    const done = await subscriptionApi({ loginPolls: 1 })
    await done.aiCliLoginStart("claude")
    // Act
    const first = mount(done)
    const row = await claudeRow()
    // Assert: 로그인됨으로 갱신, 대기 표시 없음
    expect(await row.findByText("user@example.com · max")).toBeInTheDocument()
    expect(row.queryByText(WAITING)).not.toBeInTheDocument()
    first.unmount()

    // Arrange 2: failed
    const failed = await subscriptionApi()
    vi.spyOn(failed, "aiCliLoginStatus").mockResolvedValueOnce({ state: "failed", errorCode: "login_no_url" })
    // Act 2
    mount(failed)
    const row2 = await claudeRow()
    // Assert 2
    expect(await row2.findByRole("alert")).toHaveTextContent("로그인 주소를 받지 못했습니다")
    expect(row2.getByRole("button", { name: LOGIN_BTN })).toBeInTheDocument()
  })

  it("T-ui-aicli-10 [Concurrency] recheck that reveals an existing login cancels the stale waiting session and stops polling", async () => {
    // Arrange
    const api = await subscriptionApi({ loginPolls: 5 })
    const cancel = vi.spyOn(api, "aiCliLoginCancel")
    const status = vi.spyOn(api, "aiCliLoginStatus")
    const user = userEvent.setup()
    fakeIntervals()
    mount(api)
    const row = await claudeRow()
    await user.click(await row.findByRole("button", { name: LOGIN_BTN }))
    await row.findByText(WAITING)
    // 컴포넌트 밖에서 로그인이 끝난 상황 (다른 경로로 로그인됨)
    for (let i = 0; i < 5; i++) await api.aiCliLoginStatus("claude")
    status.mockClear()

    // Act
    await user.click(row.getByRole("button", { name: RECHECK_BTN }))

    // Assert
    expect(await row.findByText("user@example.com · max")).toBeInTheDocument()
    await waitFor(() => expect(cancel).toHaveBeenCalledWith("claude"))
    expect(row.queryByText(WAITING)).not.toBeInTheDocument()
    await tick()
    await tick()
    expect(status).not.toHaveBeenCalled()
  })

  it("T-ui-aicli-11 [Business rule] the auth-mode radios are disabled while a login is waiting and enabled again after cancel", async () => {
    // Arrange
    const api = await subscriptionApi({ loginPolls: 5 })
    const user = userEvent.setup()
    fakeIntervals()
    mount(api)
    const row = await claudeRow()
    expect(row.getAllByRole("radio").every((r) => !r.hasAttribute("disabled"))).toBe(true)

    // Act
    await user.click(await row.findByRole("button", { name: LOGIN_BTN }))
    await row.findByText(WAITING)
    // Assert
    for (const r of row.getAllByRole("radio")) expect(r).toBeDisabled()

    // Act 2
    await user.click(row.getByRole("button", { name: "Claude 로그인 취소" }))
    await row.findByRole("button", { name: LOGIN_BTN })
    // Assert 2
    await waitFor(() => {
      for (const r of row.getAllByRole("radio")) expect(r).toBeEnabled()
    })
  })

  it("T-ui-aicli-08 [Business rule] usage table labels rows by authMode (null = API 키, subscription = 구독)", async () => {
    // Arrange
    const api = createMockApi({ count: 10 })
    vi.spyOn(api, "listAiUsage").mockResolvedValue([
      { day: "2026-11-07", provider: "claude", model: "haiku", requests: 2, failures: 0, inputTokens: 10, outputTokens: 5, images: 2, authMode: "subscription" },
      { day: "2026-11-07", provider: "openai", model: "gpt", requests: 1, failures: 0, inputTokens: 10, outputTokens: 5, images: 1, authMode: null },
    ])
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    // Act
    render(
      <QueryClientProvider client={queryClient}>
        <ApiProvider api={api}>
          <UsageSection />
        </ApiProvider>
      </QueryClientProvider>,
    )
    // Assert
    const table = await screen.findByRole("table")
    expect(within(table).getByRole("columnheader", { name: "인증" })).toBeInTheDocument()
    expect(within(within(table).getByText("haiku").closest("tr") as HTMLElement).getByText("구독")).toBeInTheDocument()
    expect(within(within(table).getByText("gpt").closest("tr") as HTMLElement).getByText("API 키")).toBeInTheDocument()
  })
})
