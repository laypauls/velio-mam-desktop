// Categories: Happy (T-ui-remote-11 host key flow, 16 kinds) / Validation (10 inline errors, 14 path map, 15 duplicate name) / Business rule (12 secret handling, 13 delete refusal) / Boundary (14 min one row)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider, type UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { MOCK_SERVER_FINGERPRINT } from "@/lib/api/mock-remote"
import type { RemoteSourceInput } from "@/lib/ipc-contract"
import { stubLayout } from "@/test/layout"
import { RemoteSourcesDialog } from "./RemoteSourcesDialog"

type User = ReturnType<typeof userEvent.setup>
type Api = ReturnType<typeof createMockApi>

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const SECRET = "s3cr3t-pw"
const input = (patch: Partial<RemoteSourceInput> = {}): RemoteSourceInput => ({
  kind: "ddp_audit",
  name: "ddp-1",
  host: "10.0.0.5",
  port: 22,
  user: "audit",
  authKind: "password",
  secret: SECRET,
  hostKey: MOCK_SERVER_FINGERPRINT,
  logDir: null,
  dbPath: null,
  pollSec: 3,
  pathMap: [{ from: "/", to: "F:\\UHD" }],
  ...patch,
})

function Host({ api, onCreated }: { api: UiApi; onCreated?: (id: number) => void }) {
  const [client] = useState(() => new QueryClient({ defaultOptions: { queries: { retry: false } } }))
  return (
    <ApiProvider api={api}>
      <QueryClientProvider client={client}>
        <RemoteSourcesDialog open onOpenChange={() => undefined} initialKind="ddp_audit" onCreated={(s) => onCreated?.(s.sourceId)} />
      </QueryClientProvider>
    </ApiProvider>
  )
}

async function setup(prepare?: (api: Api) => Promise<void>, onCreated?: (id: number) => void) {
  stubLayout()
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  const user = userEvent.setup()
  const api = createMockApi({ count: 10 })
  await prepare?.(api)
  render(<Host api={api} onCreated={onCreated} />)
  const dialog = await screen.findByRole("dialog", { name: "원격 스토리지 접속" })
  return { user, api, dialog }
}

const field = (dialog: HTMLElement, label: string) => within(dialog).getByLabelText(label)
const passwordInput = (dialog: HTMLElement) => within(dialog).getByLabelText("비밀번호", { selector: "input[type=password]" })

async function fillBasics(user: User, dialog: HTMLElement) {
  await user.type(field(dialog, "이름"), "ddp-1")
  await user.type(field(dialog, "호스트"), "10.0.0.5")
  await user.type(field(dialog, "계정"), "audit")
  await user.type(passwordInput(dialog), SECRET)
  await user.type(within(dialog).getByLabelText("1번째 이 PC 경로"), "F:\\UHD")
}

describe("원격 스토리지 접속 Dialog", () => {
  it("T-ui-remote-10 [Validation] empty state first; submitting an empty form shows inline errors (aria-invalid + described) and calls nothing", async () => {
    // Arrange
    const { user, api, dialog } = await setup()
    const spy = vi.spyOn(api, "createRemoteSource")
    expect(await within(dialog).findByText("등록된 접속이 없습니다")).toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "새 접속" }))
    // Act
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    // Assert
    for (const [label, message] of [
      ["이름", "이름을 1~100자로 입력하세요"],
      ["호스트", "호스트를 공백 없이 입력하세요"],
      ["계정", "계정을 입력하세요"],
    ] as const) {
      expect(field(dialog, label)).toHaveAttribute("aria-invalid", "true")
      expect(field(dialog, label)).toHaveAccessibleDescription(message)
    }
    expect(passwordInput(dialog)).toHaveAccessibleDescription("비밀번호 또는 키를 입력하세요")
    expect(within(dialog).getByLabelText("1번째 이 PC 경로")).toHaveAttribute("aria-invalid", "true")
    expect(spy).not.toHaveBeenCalled()
  })

  it("T-ui-remote-11 [Happy] host key flow: first test shows the fingerprint without a secret, an explicit confirm pins it, the second test connects, save sends the pinned key", async () => {
    // Arrange
    const { user, api, dialog } = await setup()
    const test = vi.spyOn(api, "testRemoteSource")
    const create = vi.spyOn(api, "createRemoteSource")
    await user.click(await within(dialog).findByRole("button", { name: "새 접속" }))
    await fillBasics(user, dialog)
    // Act 1: 첫 시험
    await user.click(within(dialog).getByRole("button", { name: "접속 시험" }))
    // Assert 1: 지문만 보여 주고, 비밀은 보내지 않았다
    expect(await within(dialog).findByText("호스트 키 지문을 아직 확인하지 않았습니다. 아래 지문을 확인하세요.")).toBeInTheDocument()
    expect(within(dialog).getByText(MOCK_SERVER_FINGERPRINT)).toBeInTheDocument()
    expect(test.mock.calls[0][1]).toMatchObject({ hostKey: null, secret: null })
    // Act 2: 명시적 확인 → 다시 시험
    await user.click(within(dialog).getByRole("button", { name: "이 지문을 신뢰" }))
    expect(within(dialog).queryByRole("button", { name: "이 지문을 신뢰" })).not.toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "접속 시험" }))
    // Assert 2
    expect(await within(dialog).findByText("연결됨 — 감사 로그 파일 3개 확인")).toBeInTheDocument()
    expect(test.mock.calls[1][1]).toMatchObject({ hostKey: MOCK_SERVER_FINGERPRINT, secret: SECRET })
    // Act 3: 저장
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    await waitFor(() => expect(create).toHaveBeenCalledTimes(1))
    expect(create.mock.calls[0][0]).toMatchObject({ kind: "ddp_audit", name: "ddp-1", hostKey: MOCK_SERVER_FINGERPRINT, logDir: null, dbPath: null })
    // 목록으로 돌아오고, 목록·읽기 모델에는 비밀이 없다
    expect(await within(dialog).findByText("ddp-1")).toBeInTheDocument()
    expect(JSON.stringify(await api.listRemoteSources())).not.toContain(SECRET)
    expect(dialog).not.toHaveTextContent(SECRET)
  })

  it("T-ui-remote-11b [External] a different server fingerprint warns and replaces the pin only after an explicit action; changing the host drops the pin", async () => {
    // Arrange: 다른 지문이 저장된 접속
    const other = `SHA256:${"b".repeat(43)}`
    const { user, dialog } = await setup(async (a) => void (await a.createRemoteSource(input({ hostKey: other }))))
    await user.click(await within(dialog).findByRole("button", { name: "ddp-1 수정" }))
    // Act
    await user.click(within(dialog).getByRole("button", { name: "접속 시험" }))
    // Assert: 조용히 받아들이지 않는다
    expect(await within(dialog).findByText(/저장된 지문과 다릅니다/)).toBeInTheDocument()
    expect(within(dialog).getAllByText(other).length).toBeGreaterThan(0)
    await user.click(within(dialog).getByRole("button", { name: "이 지문으로 바꾸기" }))
    expect(within(dialog).getByText(MOCK_SERVER_FINGERPRINT)).toBeInTheDocument()
    // Act 2: 호스트를 바꾸면 확인한 지문은 버려진다
    await user.type(field(dialog, "호스트"), "9")
    expect(within(dialog).getByText(/아직 확인하지 않았습니다 — \[접속 시험\]/)).toBeInTheDocument()
  })

  it("T-ui-remote-12 [Business rule] a saved secret is never prefilled; leaving it empty keeps it (null), the field is masked, and a new auth kind needs a new secret", async () => {
    // Arrange
    const { user, api, dialog } = await setup(async (a) => void (await a.createRemoteSource(input())))
    const update = vi.spyOn(api, "updateRemoteSource")
    await user.click(await within(dialog).findByRole("button", { name: "ddp-1 수정" }))
    // Assert: 비어 있고 "저장됨" 만 보인다
    const pw = passwordInput(dialog)
    expect(pw).toHaveValue("")
    expect(pw).toHaveAttribute("type", "password")
    expect(pw).toHaveAttribute("placeholder", "저장됨")
    expect(within(dialog).getByText(/저장됨 — 바꾸려면 새로 입력하세요/)).toBeInTheDocument()
    expect(dialog).not.toHaveTextContent(SECRET)
    expect(within(dialog).getByRole("radio", { name: "DDP 감사 로그" })).toBeDisabled()
    // Act 1: 비밀 없이 저장 → 저장된 비밀 유지
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1))
    expect(update.mock.calls[0][1].secret).toBeNull()
    // Act 2: 다시 열어 인증 방식을 개인 키로 바꾸면 새 비밀이 필요하다
    await user.click(await within(dialog).findByRole("button", { name: "ddp-1 수정" }))
    await user.click(within(dialog).getByRole("radio", { name: "개인 키" }))
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    // Assert
    const key = within(dialog).getByLabelText("개인 키 (PEM)")
    expect(key).toHaveAttribute("aria-invalid", "true")
    expect(key).toHaveAccessibleDescription(expect.stringContaining("비밀번호 또는 키를 입력하세요"))
    expect(update).toHaveBeenCalledTimes(1)
  })

  it("T-ui-remote-13 [Business rule] delete asks first; a connection used by a folder is refused inline and stays; an unused one is removed", async () => {
    // Arrange: ddp-1 은 폴더 1 이 쓴다, ddp-2 는 안 쓴다
    const { user, api, dialog } = await setup(async (a) => {
      const used = await a.createRemoteSource(input())
      await a.createRemoteSource(input({ name: "ddp-2" }))
      await a.setFolderWatchMode(1, "ddp_audit", { source: used.sourceId })
    })
    const del = vi.spyOn(api, "deleteRemoteSource")
    // Act 1: 쓰는 접속 삭제 시도
    await user.click(await within(dialog).findByRole("button", { name: "ddp-1 삭제" }))
    const alert = await screen.findByRole("alertdialog")
    await user.click(within(alert).getByRole("button", { name: "삭제" }))
    // Assert 1: 거부 사유를 같은 자리에 보여 주고 닫지 않는다
    expect(await within(alert).findByText(/이 접속을 쓰는 폴더가 있어 삭제할 수 없습니다/)).toHaveAttribute("role", "alert")
    expect(screen.getByRole("alertdialog")).toBeInTheDocument()
    await user.click(within(alert).getByRole("button", { name: "취소" }))
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument())
    expect(within(dialog).getByText("ddp-1")).toBeInTheDocument()
    // Act 2: 안 쓰는 접속 삭제
    await user.click(within(dialog).getByRole("button", { name: "ddp-2 삭제" }))
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "삭제" }))
    // Assert 2
    await waitFor(() => expect(within(dialog).queryByText("ddp-2")).not.toBeInTheDocument())
    expect(del).toHaveBeenCalledTimes(2)
    expect(within(dialog).getByText("ddp-1")).toBeInTheDocument()
  })

  it("T-ui-remote-14 [Boundary] path map rows: at least one row, add/remove, and a non-absolute PC path is rejected inline", async () => {
    // Arrange
    const { user, dialog } = await setup()
    await user.click(await within(dialog).findByRole("button", { name: "새 접속" }))
    const remove1 = () => within(dialog).getByRole("button", { name: "1번째 경로 변환 삭제" })
    // Assert: 1줄이면 삭제할 수 없다
    expect(remove1()).toBeDisabled()
    // Act: 줄 추가 → 삭제 가능
    await user.click(within(dialog).getByRole("button", { name: "경로 추가" }))
    expect(within(dialog).getByLabelText("2번째 스토리지 경로")).toBeInTheDocument()
    expect(remove1()).toBeEnabled()
    await user.click(within(dialog).getByRole("button", { name: "2번째 경로 변환 삭제" }))
    expect(within(dialog).queryByLabelText("2번째 스토리지 경로")).not.toBeInTheDocument()
    // Act 2: 상대 경로
    await user.type(within(dialog).getByLabelText("1번째 이 PC 경로"), "relative\\path")
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    // Assert 2
    expect(within(dialog).getByRole("group", { name: "경로 변환" })).toHaveAccessibleDescription(expect.stringContaining("절대 경로"))
  })

  it("T-ui-remote-15 [Validation] a server-side duplicate name lands on the name field, the form stays open", async () => {
    // Arrange: 같은 이름이 이미 있다
    const { user, dialog } = await setup(async (a) => void (await a.createRemoteSource(input())))
    await user.click(await within(dialog).findByRole("button", { name: "새 접속" }))
    await fillBasics(user, dialog)
    // Act
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    // Assert
    await waitFor(() => expect(field(dialog, "이름")).toHaveAttribute("aria-invalid", "true"))
    expect(field(dialog, "이름")).toHaveAccessibleDescription("같은 이름의 접속이 이미 있습니다")
    expect(within(dialog).getByRole("button", { name: "접속 시험" })).toBeInTheDocument()
  })

  it("T-ui-remote-16 [Happy] the Linux queue kind shows the DB path (not the log folder), tests with row count, and reports the created connection", async () => {
    // Arrange
    const created: number[] = []
    const { user, dialog } = await setup(undefined, (id) => created.push(id))
    await user.click(await within(dialog).findByRole("button", { name: "새 접속" }))
    // Act
    await user.click(within(dialog).getByRole("radio", { name: "리눅스 스토리지 이벤트 큐" }))
    // Assert: 종류에 따라 필드가 바뀐다
    expect(within(dialog).getByLabelText("이벤트 DB 경로")).toBeInTheDocument()
    expect(within(dialog).queryByLabelText("감사 로그 폴더")).not.toBeInTheDocument()
    expect(within(dialog).getByLabelText("1번째 스토리지 경로")).toHaveValue("/data/incoming")
    // Act 2: 채우고 두 번 시험 + 저장
    await user.type(field(dialog, "이름"), "ses-1")
    await user.type(field(dialog, "호스트"), "10.0.0.9")
    await user.type(field(dialog, "계정"), "root")
    await user.type(passwordInput(dialog), SECRET)
    await user.type(field(dialog, "이벤트 DB 경로"), "/srv/ses/events.db")
    await user.type(within(dialog).getByLabelText("1번째 이 PC 경로"), "\\\\nas\\media")
    await user.click(within(dialog).getByRole("button", { name: "접속 시험" }))
    await user.click(await within(dialog).findByRole("button", { name: "이 지문을 신뢰" }))
    await user.click(within(dialog).getByRole("button", { name: "접속 시험" }))
    expect(await within(dialog).findByText("연결됨 — 이벤트 3건 (마지막 id 3)")).toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    // Assert
    await waitFor(() => expect(created).toEqual([1]))
  })
})
