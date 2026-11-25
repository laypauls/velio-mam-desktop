// Categories: Happy (T-ui-umeta-01~03, 09, 12, 16) / Validation (T-ui-umeta-04~07) / Business rule (T-ui-umeta-08, 10, 11, 13) / External (T-ui-umeta-14, 15)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { MAX_METADATA_FIELDS, type MetadataField, type MetadataFieldOption, type MetadataValueType } from "@/lib/ipc-contract-metadata"
import * as ipc from "@/lib/ipc-metadata"
import { toast } from "sonner"
import { MetadataFieldsTab } from "./MetadataFieldsTab"

vi.mock("@/lib/ipc-metadata")
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }))

const m = vi.mocked(ipc)
type User = ReturnType<typeof userEvent.setup>

function opt(optionId: number, label: string, position: number): MetadataFieldOption {
  return { optionId, label, position }
}
function field(fieldId: number, name: string, valueType: MetadataValueType = "keyword", options: MetadataFieldOption[] = []): MetadataField {
  return { fieldId, name, valueType, position: fieldId, options, createdAt: 0, updatedAt: 0 }
}

/** 서버 상태를 흉내 내는 목록 — 변경 command 가 이 배열을 바꾸고 list 가 현재 값을 돌려준다 */
let server: MetadataField[]

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  server = []
  m.listMetadataFields.mockImplementation(() => Promise.resolve(structuredClone(server)))
  m.createMetadataField.mockImplementation((name, valueType, options) => {
    const created = field(server.length + 1, name, valueType, options.map((label, i) => opt(100 + i, label, i)))
    server = [...server, created]
    return Promise.resolve(created)
  })
  m.renameMetadataField.mockImplementation((fieldId, name) => {
    server = server.map((f) => (f.fieldId === fieldId ? { ...f, name } : f))
    return Promise.resolve(server.find((f) => f.fieldId === fieldId) as MetadataField)
  })
  m.reorderMetadataFields.mockImplementation((ids) => {
    server = ids.map((id) => server.find((f) => f.fieldId === id) as MetadataField)
    return Promise.resolve()
  })
  m.deleteMetadataField.mockImplementation((fieldId) => {
    server = server.filter((f) => f.fieldId !== fieldId)
    return Promise.resolve(7)
  })
  m.metadataFieldValueCount.mockResolvedValue(7)
  m.metadataOptionUsage.mockResolvedValue(3)
  m.addMetadataOption.mockImplementation((fieldId, label) => {
    const added = opt(500, label, 9)
    server = server.map((f) => (f.fieldId === fieldId ? { ...f, options: [...f.options, added] } : f))
    return Promise.resolve(added)
  })
  m.deleteMetadataOption.mockImplementation((optionId) => {
    server = server.map((f) => ({ ...f, options: f.options.filter((o) => o.optionId !== optionId) }))
    return Promise.resolve(3)
  })
})

afterEach(() => {
  vi.resetAllMocks()
})

function renderTab() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
  return render(
    <QueryClientProvider client={queryClient}>
      <MetadataFieldsTab />
    </QueryClientProvider>,
  )
}

async function openRowMenu(user: User, name: string, item: "편집" | "삭제") {
  await user.click(await screen.findByRole("button", { name: `${name} 작업` }))
  await user.click(await screen.findByRole("menuitem", { name: item }))
}

async function chooseType(user: User, dialog: HTMLElement, label: string) {
  await user.click(within(dialog).getByRole("combobox", { name: "타입" }))
  await user.click(await screen.findByRole("option", { name: label }))
}

describe("settings > 메타데이터 필드 (custom metadata step 6)", () => {
  it("T-ui-umeta-01 [Happy] lists fields with name, type label and option count", async () => {
    // Arrange
    server = [field(1, "프로젝트", "enum", [opt(1, "봄", 0), opt(2, "여름", 1)]), field(2, "촬영 감독"), field(3, "납품일", "date")]
    // Act
    renderTab()
    // Assert
    const table = await screen.findByRole("table", { name: "메타데이터 필드" })
    const rows = within(table).getAllByRole("row").slice(1)
    expect(rows).toHaveLength(3)
    expect(within(rows[0]).getByText("프로젝트")).toBeInTheDocument()
    expect(within(rows[0]).getByText("선택")).toBeInTheDocument()
    expect(within(rows[0]).getByText("2개")).toBeInTheDocument()
    expect(within(rows[1]).getByText("짧은 글")).toBeInTheDocument()
    expect(within(rows[2]).getByText("날짜")).toBeInTheDocument()
    expect(screen.getByText(/모든 필드는 필터로 쓸 수 있고/)).toBeInTheDocument()
  })

  it("T-ui-umeta-02 [Happy] empty state shows title, description and an action that opens the add dialog", async () => {
    // Arrange / Act
    const user = userEvent.setup()
    renderTab()
    // Assert
    expect(await screen.findByText("아직 만든 필드가 없습니다")).toBeInTheDocument()
    const action = screen.getAllByRole("button", { name: "필드 추가" })
    expect(action).toHaveLength(2)
    await user.click(action[1])
    expect(await screen.findByRole("dialog", { name: "필드 추가" })).toBeInTheDocument()
  })

  it("T-ui-umeta-03 [Happy] create keyword field calls the command, closes and shows the new row", async () => {
    // Arrange
    const user = userEvent.setup()
    renderTab()
    await user.click((await screen.findAllByRole("button", { name: "필드 추가" }))[0])
    const dialog = await screen.findByRole("dialog", { name: "필드 추가" })
    // Act
    await user.type(within(dialog).getByRole("textbox", { name: "이름" }), "  촬영 감독 ")
    await user.click(within(dialog).getByRole("button", { name: "만들기" }))
    // Assert
    await waitFor(() => expect(m.createMetadataField).toHaveBeenCalledWith("촬영 감독", "keyword", []))
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(await screen.findByRole("cell", { name: "촬영 감독" })).toBeInTheDocument()
  })

  it("T-ui-umeta-04 [Validation] enum create needs at least one option, then sends options in order", async () => {
    // Arrange
    const user = userEvent.setup()
    renderTab()
    await user.click((await screen.findAllByRole("button", { name: "필드 추가" }))[0])
    const dialog = await screen.findByRole("dialog", { name: "필드 추가" })
    await user.type(within(dialog).getByRole("textbox", { name: "이름" }), "프로젝트")
    await chooseType(user, dialog, "선택")
    // Act 1: 선택지 없이 제출
    await user.click(within(dialog).getByRole("button", { name: "만들기" }))
    // Assert 1
    expect(await within(dialog).findByText("선택지를 하나 이상 추가하세요.")).toBeInTheDocument()
    expect(m.createMetadataField).not.toHaveBeenCalled()
    // Act 2: 선택지 둘 추가(Enter) 후 위/아래로 순서 바꾸기
    const add = within(dialog).getByRole("textbox", { name: "새 선택지 이름" })
    await user.type(add, "봄{Enter}")
    await user.type(add, "여름{Enter}")
    await user.click(within(dialog).getByRole("button", { name: "여름 위로 이동" }))
    await user.click(within(dialog).getByRole("button", { name: "만들기" }))
    // Assert 2
    await waitFor(() => expect(m.createMetadataField).toHaveBeenCalledWith("프로젝트", "enum", ["여름", "봄"]))
  })

  it("T-ui-umeta-05 [Validation] empty name and duplicate name show inline errors wired by aria-invalid/aria-describedby", async () => {
    // Arrange
    server = [field(1, "프로젝트")]
    const user = userEvent.setup()
    renderTab()
    await user.click((await screen.findAllByRole("button", { name: "필드 추가" }))[0])
    const dialog = await screen.findByRole("dialog", { name: "필드 추가" })
    const input = within(dialog).getByRole("textbox", { name: "이름" })
    // Act 1: 빈 이름
    await user.click(within(dialog).getByRole("button", { name: "만들기" }))
    // Assert 1
    const empty = await within(dialog).findByText("이름을 입력하세요.")
    expect(input).toHaveAttribute("aria-invalid", "true")
    expect(input).toHaveAttribute("aria-describedby", empty.id)
    // Act 2: 대소문자·공백만 다른 중복
    await user.type(input, " 프로젝트 ")
    await user.click(within(dialog).getByRole("button", { name: "만들기" }))
    // Assert 2
    const dup = await within(dialog).findByText("같은 이름의 필드가 이미 있습니다.")
    expect(input).toHaveAttribute("aria-describedby", dup.id)
    expect(m.createMetadataField).not.toHaveBeenCalled()
  })

  it("T-ui-umeta-06 [External] backend already_registered shows the duplicate error inline and keeps the dialog open", async () => {
    // Arrange: 목록이 아직 모르는 이름이 서버에서는 중복
    m.createMetadataField.mockRejectedValueOnce({ code: "already_registered", message: "dup" })
    const user = userEvent.setup()
    renderTab()
    await user.click((await screen.findAllByRole("button", { name: "필드 추가" }))[0])
    const dialog = await screen.findByRole("dialog", { name: "필드 추가" })
    // Act
    await user.type(within(dialog).getByRole("textbox", { name: "이름" }), "새 이름")
    await user.click(within(dialog).getByRole("button", { name: "만들기" }))
    // Assert
    const err = await within(dialog).findByText("같은 이름의 필드가 이미 있습니다.")
    expect(within(dialog).getByRole("textbox", { name: "이름" })).toHaveAttribute("aria-describedby", err.id)
    expect(screen.getByRole("dialog", { name: "필드 추가" })).toBeInTheDocument()
  })

  it("T-ui-umeta-07 [Validation] an empty option label blocks creation with a row error; adding an empty option is rejected too", async () => {
    // Arrange
    const user = userEvent.setup()
    renderTab()
    await user.click((await screen.findAllByRole("button", { name: "필드 추가" }))[0])
    const dialog = await screen.findByRole("dialog", { name: "필드 추가" })
    await user.type(within(dialog).getByRole("textbox", { name: "이름" }), "프로젝트")
    await chooseType(user, dialog, "다중 선택")
    // Act 1: 빈 선택지 추가 시도
    await user.click(within(dialog).getByRole("button", { name: "추가" }))
    // Assert 1
    const addInput = within(dialog).getByRole("textbox", { name: "새 선택지 이름" })
    expect(await within(dialog).findByText("선택지 이름을 입력하세요.")).toBeInTheDocument()
    expect(addInput).toHaveAttribute("aria-invalid", "true")
    // Act 2: 추가한 선택지를 비운 채 제출
    await user.type(addInput, "봄{Enter}")
    await user.clear(within(dialog).getByRole("textbox", { name: "선택지 1 이름" }))
    await user.click(within(dialog).getByRole("button", { name: "만들기" }))
    // Assert 2
    const row = within(dialog).getByRole("textbox", { name: "선택지 1 이름" })
    await waitFor(() => expect(row).toHaveAttribute("aria-invalid", "true"))
    expect(row).toHaveAccessibleDescription("선택지 이름을 입력하세요.")
    expect(m.createMetadataField).not.toHaveBeenCalled()
  })

  it("T-ui-umeta-08 [Business rule] edit disables the type select and renames through the command", async () => {
    // Arrange
    server = [field(1, "촬영 감독"), field(2, "납품일", "date")]
    const user = userEvent.setup()
    renderTab()
    await openRowMenu(user, "납품일", "편집")
    const dialog = await screen.findByRole("dialog", { name: "필드 편집" })
    // Assert: 타입 비활성
    const type = within(dialog).getByRole("combobox", { name: "타입" })
    expect(type).toBeDisabled()
    expect(type).toHaveTextContent("날짜")
    // Act
    const name = within(dialog).getByRole("textbox", { name: "이름" })
    await user.clear(name)
    await user.type(name, "납품 예정일")
    await user.click(within(dialog).getByRole("button", { name: "저장" }))
    // Assert
    await waitFor(() => expect(m.renameMetadataField).toHaveBeenCalledWith(2, "납품 예정일"))
    expect(await screen.findByRole("cell", { name: "납품 예정일" })).toBeInTheDocument()
    expect(m.createMetadataField).not.toHaveBeenCalled()
  })

  it("T-ui-umeta-09 [Boundary] at the field cap the add button is disabled and the limit is explained", async () => {
    // Arrange
    server = Array.from({ length: MAX_METADATA_FIELDS }, (_, i) => field(i + 1, `필드 ${i + 1}`))
    // Act
    renderTab()
    // Assert
    await screen.findByText(/최대 20개까지 만들 수 있습니다/)
    const add = screen.getByRole("button", { name: "필드 추가" })
    expect(add).toBeDisabled()
    expect(add).toHaveAccessibleDescription(/최대 20개까지 만들 수 있습니다/)
    expect(screen.getByText("20 / 20개")).toBeInTheDocument()
  })

  it("T-ui-umeta-10 [Business rule] deleting a field shows the value count, calls the command and the row disappears", async () => {
    // Arrange
    server = [field(1, "촬영 감독"), field(2, "납품일", "date")]
    const user = userEvent.setup()
    renderTab()
    await openRowMenu(user, "납품일", "삭제")
    const alert = await screen.findByRole("alertdialog", { name: "필드 삭제" })
    // Assert: 값 건수와 복원 불가 안내
    expect(await within(alert).findByText("값 7건이 함께 삭제되며 복원할 수 없습니다.")).toBeInTheDocument()
    expect(m.metadataFieldValueCount).toHaveBeenCalledWith(2)
    // Act
    await user.click(within(alert).getByRole("button", { name: "삭제" }))
    // Assert
    await waitFor(() => expect(m.deleteMetadataField).toHaveBeenCalledWith(2))
    await waitFor(() => expect(screen.queryByRole("cell", { name: "납품일" })).not.toBeInTheDocument())
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
  })

  it("T-ui-umeta-11 [Business rule] deleting an option asks with the usage count before calling the command", async () => {
    // Arrange
    server = [field(1, "프로젝트", "enum", [opt(10, "봄", 0), opt(11, "여름", 1)])]
    const user = userEvent.setup()
    renderTab()
    await openRowMenu(user, "프로젝트", "편집")
    const dialog = await screen.findByRole("dialog", { name: "필드 편집" })
    // Act 1
    await user.click(within(dialog).getByRole("button", { name: "봄 삭제" }))
    const alert = await screen.findByRole("alertdialog", { name: "선택지 삭제" })
    // Assert 1: 사용 건수, 아직 호출 전
    expect(await within(alert).findByText(/사용 중 3건의 값이 지워집니다/)).toBeInTheDocument()
    expect(m.metadataOptionUsage).toHaveBeenCalledWith(10)
    expect(m.deleteMetadataOption).not.toHaveBeenCalled()
    // Act 2
    await user.click(within(alert).getByRole("button", { name: "삭제" }))
    // Assert 2
    await waitFor(() => expect(m.deleteMetadataOption).toHaveBeenCalledWith(10))
    await waitFor(() => expect(within(dialog).queryByRole("textbox", { name: "선택지 2 이름" })).not.toBeInTheDocument())
    expect(within(dialog).getByRole("textbox", { name: "선택지 1 이름" })).toHaveValue("여름")
  })

  it("T-ui-umeta-12 [Happy] reorder buttons send the full new id order; first up / last down are disabled", async () => {
    // Arrange
    server = [field(1, "A"), field(2, "B"), field(3, "C")]
    const user = userEvent.setup()
    renderTab()
    expect(await screen.findByRole("button", { name: "A 위로 이동" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "C 아래로 이동" })).toBeDisabled()
    // Act
    await user.click(screen.getByRole("button", { name: "A 아래로 이동" }))
    // Assert
    await waitFor(() => expect(m.reorderMetadataFields).toHaveBeenCalledWith([2, 1, 3]))
    await waitFor(() => expect(within(screen.getAllByRole("row")[1]).getByText("B")).toBeInTheDocument())
  })

  it("T-ui-umeta-13 [Business rule] edit-mode option add rejects a duplicate label without calling the backend, and adds a new one", async () => {
    // Arrange
    server = [field(1, "프로젝트", "enum", [opt(10, "봄", 0)])]
    const user = userEvent.setup()
    renderTab()
    await openRowMenu(user, "프로젝트", "편집")
    const dialog = await screen.findByRole("dialog", { name: "필드 편집" })
    const add = within(dialog).getByRole("textbox", { name: "새 선택지 이름" })
    // Act 1: 중복
    await user.type(add, "봄{Enter}")
    // Assert 1
    expect(await within(dialog).findByText("같은 이름의 선택지가 있습니다.")).toBeInTheDocument()
    expect(m.addMetadataOption).not.toHaveBeenCalled()
    // Act 2
    await user.clear(add)
    await user.type(add, "가을{Enter}")
    // Assert 2
    await waitFor(() => expect(m.addMetadataOption).toHaveBeenCalledWith(1, "가을"))
    expect(await within(dialog).findByRole("textbox", { name: "선택지 2 이름" })).toHaveValue("가을")
  })

  it("T-ui-umeta-14 [External] a failed reorder shows a toast", async () => {
    // Arrange
    server = [field(1, "A"), field(2, "B")]
    m.reorderMetadataFields.mockRejectedValueOnce({ code: "invalid_input", message: "bad" })
    const user = userEvent.setup()
    renderTab()
    // Act
    await user.click(await screen.findByRole("button", { name: "A 아래로 이동" }))
    // Assert
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("입력값이 올바르지 않습니다."))
  })

  it("T-ui-umeta-15 [External] a failed field delete keeps the confirm open with an inline error; a failed load offers retry", async () => {
    // Arrange
    server = [field(1, "A")]
    m.deleteMetadataField.mockRejectedValueOnce({ code: "not_found", message: "gone" })
    const user = userEvent.setup()
    renderTab()
    await openRowMenu(user, "A", "삭제")
    const alert = await screen.findByRole("alertdialog", { name: "필드 삭제" })
    // Act
    await user.click(await within(alert).findByRole("button", { name: "삭제" }))
    // Assert
    expect(await within(alert).findByRole("alert")).toHaveTextContent("대상을 찾을 수 없습니다.")
    expect(screen.getByRole("alertdialog", { name: "필드 삭제" })).toBeInTheDocument()
  })

  it("T-ui-umeta-16 [External] load failure shows an error with a retry that refetches", async () => {
    // Arrange
    m.listMetadataFields.mockRejectedValueOnce({ code: "internal", message: "x" })
    server = [field(1, "복구됨")]
    const user = userEvent.setup()
    renderTab()
    // Act
    expect(await screen.findByText("메타데이터 필드를 불러오지 못했습니다.")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "다시 시도" }))
    // Assert
    expect(await screen.findByRole("cell", { name: "복구됨" })).toBeInTheDocument()
  })
})
