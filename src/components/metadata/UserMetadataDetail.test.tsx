// Categories: Happy (T-ui-umeta-detail-01, 02, 05, 07, 08, 12, 13) / Validation (T-ui-umeta-detail-03, 04) / Business rule (T-ui-umeta-detail-06, 10, 14, 15) / External (T-ui-umeta-detail-09, 11, 16)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { AssetMetadataSection } from "@/components/detail/AssetMetadataSection"
import { MAX_BULK_ASSETS, type AssetMetadataValue, type MetadataField, type MetadataValueType } from "@/lib/ipc-contract-metadata"
import * as ipc from "@/lib/ipc-metadata"
import { BulkMetadataPanel } from "./BulkMetadataPanel"

vi.mock("@/lib/ipc-metadata")
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }))

const m = vi.mocked(ipc)
type User = ReturnType<typeof userEvent.setup>

const OPTIONS = [
  { optionId: 11, label: "봄 캠페인", position: 0 },
  { optionId: 12, label: "여름 캠페인", position: 1 },
]

function field(fieldId: number, name: string, valueType: MetadataValueType, options = valueType === "enum" || valueType === "multiEnum" ? OPTIONS : []): MetadataField {
  return { fieldId, name, valueType, position: fieldId, options, createdAt: 0, updatedAt: 0 }
}

const F = {
  kw: field(1, "촬영 감독", "keyword"),
  num: field(2, "예산", "number"),
  date: field(3, "납품일", "date"),
  bool: field(4, "승인", "boolean"),
  en: field(5, "프로젝트", "enum"),
  multi: field(6, "채널", "multiEnum"),
}
const ALL = Object.values(F)

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  m.setAssetMetadata.mockResolvedValue(undefined)
  m.bulkSetAssetMetadata.mockResolvedValue(0)
})
afterEach(() => {
  vi.resetAllMocks()
})

function renderUi(ui: React.ReactElement) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
  return render(<QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>)
}

function setup(fields: MetadataField[], values: AssetMetadataValue[] = []) {
  m.listMetadataFields.mockResolvedValue(fields)
  m.getAssetMetadata.mockResolvedValue(values)
  return renderUi(<AssetMetadataSection assetId={7} />)
}

async function pick(user: User, fieldName: string, optionName: string) {
  await user.click(screen.getByRole("combobox", { name: fieldName }))
  await user.click(await screen.findByRole("option", { name: optionName }))
}

describe("detail panel — single asset user metadata", () => {
  it("T-ui-umeta-detail-01 [Happy] renders one input per type with the stored values", async () => {
    // Arrange + Act
    setup(ALL, [
      { fieldId: 1, value: { kind: "keyword", value: "김감독" } },
      { fieldId: 2, value: { kind: "number", value: 12.5 } },
      { fieldId: 3, value: { kind: "date", value: 20261120 } },
      { fieldId: 4, value: { kind: "boolean", value: false } },
      { fieldId: 5, value: { kind: "enum", value: 12 } },
      { fieldId: 6, value: { kind: "multiEnum", value: [11] } },
    ])

    // Assert
    expect(await screen.findByLabelText("촬영 감독")).toHaveValue("김감독")
    expect(screen.getByLabelText("예산")).toHaveValue("12.5")
    expect(screen.getByLabelText("납품일")).toHaveValue("2026-11-20")
    expect(screen.getByRole("combobox", { name: "승인" })).toHaveTextContent("아니요")
    expect(screen.getByRole("combobox", { name: "프로젝트" })).toHaveTextContent("여름 캠페인")
    const group = screen.getByRole("group", { name: "채널" })
    expect(within(group).getByRole("checkbox", { name: "봄 캠페인" })).toBeChecked()
    expect(within(group).getByRole("checkbox", { name: "여름 캠페인" })).not.toBeChecked()
    expect(m.getAssetMetadata).toHaveBeenCalledWith(7)
  })

  it("T-ui-umeta-detail-02 [Happy] keyword saves trimmed on blur; emptying it sends clear", async () => {
    // Arrange
    const user = userEvent.setup()
    setup([F.kw], [{ fieldId: 1, value: { kind: "keyword", value: "김" } }])
    const input = await screen.findByLabelText("촬영 감독")

    // Act + Assert: 값 변경 (앞뒤 공백 제거)
    await user.clear(input)
    await user.type(input, "  박감독  ")
    await user.tab()
    await waitFor(() =>
      expect(m.setAssetMetadata).toHaveBeenCalledWith(7, [{ op: "set", fieldId: 1, value: { kind: "keyword", value: "박감독" } }])
    )

    // Act + Assert: 비우면 값 없음
    await user.clear(input)
    await user.tab()
    await waitFor(() => expect(m.setAssetMetadata).toHaveBeenLastCalledWith(7, [{ op: "clear", fieldId: 1 }]))
  })

  it("T-ui-umeta-detail-03 [Validation] keyword over 200 chars shows an inline error and is not sent", async () => {
    // Arrange
    setup([F.kw])
    const input = await screen.findByLabelText("촬영 감독")

    // Act
    fireEvent.change(input, { target: { value: "가".repeat(201) } })
    fireEvent.blur(input)

    // Assert
    const alert = await screen.findByRole("alert")
    expect(alert).toHaveTextContent("최대 200자")
    expect(input).toHaveAttribute("aria-invalid", "true")
    expect(input).toHaveAttribute("aria-describedby", alert.id)
    expect(m.setAssetMetadata).not.toHaveBeenCalled()
  })

  it("T-ui-umeta-detail-04 [Validation] number accepts finite values only", async () => {
    // Arrange
    const user = userEvent.setup()
    setup([F.num])
    const input = await screen.findByLabelText("예산")

    // Act + Assert: 숫자 아님 / 무한대
    for (const bad of ["abc", "1e999", "1,5"]) {
      await user.clear(input)
      await user.type(input, bad)
      await user.tab()
      expect(await screen.findByRole("alert")).toHaveTextContent("올바른 숫자")
      expect(input).toHaveAttribute("aria-invalid", "true")
    }
    expect(m.setAssetMetadata).not.toHaveBeenCalled()

    // Act + Assert: 올바른 값은 저장되고 오류가 사라진다
    await user.clear(input)
    await user.type(input, "-12.5")
    await user.tab()
    await waitFor(() => expect(m.setAssetMetadata).toHaveBeenCalledWith(7, [{ op: "set", fieldId: 2, value: { kind: "number", value: -12.5 } }]))
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
  })

  it("T-ui-umeta-detail-05 [Happy] date maps to YYYYMMDD on blur (not while typing)", async () => {
    // Arrange
    setup([F.date])
    const input = await screen.findByLabelText("납품일")

    // Act
    fireEvent.change(input, { target: { value: "2026-11-20" } })
    expect(m.setAssetMetadata).not.toHaveBeenCalled() // 입력 중에는 저장하지 않는다
    fireEvent.blur(input)

    // Assert
    await waitFor(() => expect(m.setAssetMetadata).toHaveBeenCalledWith(7, [{ op: "set", fieldId: 3, value: { kind: "date", value: 20261120 } }]))
  })

  it("T-ui-date-09 [Validation] date text in a wrong format shows the inline error on blur and saves nothing", async () => {
    // Arrange
    setup([F.date])
    const input = await screen.findByLabelText("납품일")

    // Act
    fireEvent.change(input, { target: { value: "2026-02-30" } })
    fireEvent.blur(input)

    // Assert
    expect(await screen.findByRole("alert")).toHaveTextContent("올바른 날짜를 입력하세요.")
    expect(input).toHaveAttribute("aria-invalid", "true")
    expect(m.setAssetMetadata).not.toHaveBeenCalled()
  })

  it("T-ui-date-10 [Happy] picking a day in the calendar saves immediately as YYYYMMDD", async () => {
    // Arrange
    const user = userEvent.setup()
    setup([F.date], [{ fieldId: 3, value: { kind: "date", value: 20261120 } }])
    await screen.findByLabelText("납품일")

    // Act
    await user.click(screen.getByRole("button", { name: "달력 열기" }))
    await user.click(await screen.findByRole("button", { name: /2026년 11월 21일/ }))

    // Assert
    await waitFor(() => expect(m.setAssetMetadata).toHaveBeenCalledWith(7, [{ op: "set", fieldId: 3, value: { kind: "date", value: 20261121 } }]))
  })

  it("T-ui-umeta-detail-06 [Business rule] date clear button and enum '값 없음' sentinel send clear", async () => {
    // Arrange
    const user = userEvent.setup()
    setup([F.date, F.en], [
      { fieldId: 3, value: { kind: "date", value: 20261120 } },
      { fieldId: 5, value: { kind: "enum", value: 11 } },
    ])
    await screen.findByLabelText("납품일")

    // Act + Assert
    await user.click(screen.getByRole("button", { name: "납품일 값 지우기" }))
    await waitFor(() => expect(m.setAssetMetadata).toHaveBeenCalledWith(7, [{ op: "clear", fieldId: 3 }]))
    await pick(user, "프로젝트", "값 없음")
    await waitFor(() => expect(m.setAssetMetadata).toHaveBeenLastCalledWith(7, [{ op: "clear", fieldId: 5 }]))
  })

  it("T-ui-umeta-detail-07 [Happy] enum and boolean selections are saved immediately", async () => {
    // Arrange
    const user = userEvent.setup()
    setup([F.bool, F.en])
    await screen.findByRole("combobox", { name: "승인" })

    // Act + Assert
    await pick(user, "프로젝트", "여름 캠페인")
    await waitFor(() => expect(m.setAssetMetadata).toHaveBeenCalledWith(7, [{ op: "set", fieldId: 5, value: { kind: "enum", value: 12 } }]))
    await pick(user, "승인", "예")
    await waitFor(() => expect(m.setAssetMetadata).toHaveBeenLastCalledWith(7, [{ op: "set", fieldId: 4, value: { kind: "boolean", value: true } }]))
  })

  it("T-ui-umeta-detail-08 [Happy] multi-select toggles save the full set; unchecking the last one clears", async () => {
    // Arrange
    const user = userEvent.setup()
    setup([F.multi], [{ fieldId: 6, value: { kind: "multiEnum", value: [11] } }])
    const group = await screen.findByRole("group", { name: "채널" })

    // Act + Assert: 하나 더 선택 (선택지 순서로 정렬)
    await user.click(within(group).getByRole("checkbox", { name: "여름 캠페인" }))
    await waitFor(() => expect(m.setAssetMetadata).toHaveBeenCalledWith(7, [{ op: "set", fieldId: 6, value: { kind: "multiEnum", value: [11, 12] } }]))

    // Act + Assert: 모두 해제하면 값 없음
    await user.click(within(group).getByRole("checkbox", { name: "봄 캠페인" }))
    await user.click(within(group).getByRole("checkbox", { name: "여름 캠페인" }))
    await waitFor(() => expect(m.setAssetMetadata).toHaveBeenLastCalledWith(7, [{ op: "clear", fieldId: 6 }]))
  })

  it("T-ui-umeta-detail-09 [External] a failed save shows an inline error on that field", async () => {
    // Arrange
    const user = userEvent.setup()
    m.setAssetMetadata.mockRejectedValue({ code: "invalid_input", message: "bad" })
    setup([F.kw])
    const input = await screen.findByLabelText("촬영 감독")

    // Act
    await user.type(input, "값")
    await user.tab()

    // Assert
    const alert = await screen.findByRole("alert")
    expect(alert).toHaveTextContent(/\S/)
    expect(input).toHaveAttribute("aria-invalid", "true")
    expect(input).toHaveAttribute("aria-describedby", alert.id)
  })

  it("T-ui-umeta-detail-10 [Business rule] no fields defined hides the whole section without loading values", async () => {
    // Arrange + Act
    setup([])
    await waitFor(() => expect(m.listMetadataFields).toHaveBeenCalled())

    // Assert
    await waitFor(() => expect(screen.queryByText("사용자 메타데이터")).not.toBeInTheDocument())
    expect(m.getAssetMetadata).not.toHaveBeenCalled()
  })

  it("T-ui-umeta-detail-11 [External] a values load failure shows an alert with retry", async () => {
    // Arrange
    const user = userEvent.setup()
    m.listMetadataFields.mockResolvedValue([F.kw])
    m.getAssetMetadata.mockRejectedValueOnce({ code: "internal", message: "x" }).mockResolvedValue([{ fieldId: 1, value: { kind: "keyword", value: "복구" } }])
    renderUi(<AssetMetadataSection assetId={7} />)

    // Act
    await user.click(await screen.findByRole("button", { name: "다시 시도" }))

    // Assert
    expect(await screen.findByLabelText("촬영 감독")).toHaveValue("복구")
  })
})

describe("detail panel — multi selection bulk assign", () => {
  const ids = (n: number) => new Set(Array.from({ length: n }, (_, i) => i + 1))
  const renderBulk = (n = 3, fields: MetadataField[] = ALL) => renderUi(<BulkMetadataPanel ids={ids(n)} fields={fields} />)
  const apply = () => screen.getByRole("button", { name: "적용" })
  // 적용 버튼은 네이티브 disabled 가 아니라 aria-disabled (입력 직후 클릭이 사라지지 않게)
  const applyBlocked = () => apply().getAttribute("aria-disabled") === "true"

  it("T-ui-umeta-detail-12 [Happy] starts with every field unchanged, loads no values and keeps apply disabled", () => {
    // Arrange + Act
    renderBulk()

    // Assert
    expect(screen.getByText("3개 콘텐츠")).toBeInTheDocument()
    expect(screen.getByLabelText("촬영 감독")).toHaveValue("")
    expect(screen.getByRole("combobox", { name: "프로젝트" })).toHaveTextContent("변경 안 함")
    expect(screen.getByRole("combobox", { name: "승인" })).toHaveTextContent("변경 안 함")
    expect(applyBlocked()).toBe(true)
    expect(m.getAssetMetadata).not.toHaveBeenCalled()
  })

  it("T-ui-umeta-detail-13 [Happy] apply sends only the changed fields (set + clear) and toasts the changed count", async () => {
    // Arrange
    const user = userEvent.setup()
    m.bulkSetAssetMetadata.mockResolvedValue(2)
    renderBulk()

    // Act
    await pick(user, "프로젝트", "봄 캠페인")
    await user.type(screen.getByLabelText("촬영 감독"), "김감독")
    await user.tab()
    await user.click(screen.getByRole("button", { name: "납품일 값 지우기" }))
    expect(screen.getByText("값 지움")).toBeInTheDocument()
    await user.click(apply())

    // Assert
    await waitFor(() => expect(m.bulkSetAssetMetadata).toHaveBeenCalledTimes(1))
    const [assetIds, patches] = m.bulkSetAssetMetadata.mock.calls[0]
    expect(assetIds).toEqual([1, 2, 3])
    expect(patches).toEqual([
      { op: "set", fieldId: 1, value: { kind: "keyword", value: "김감독" } },
      { op: "clear", fieldId: 3 },
      { op: "set", fieldId: 5, value: { kind: "enum", value: 11 } },
    ])
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("2개 콘텐츠에 적용했습니다."))
    // 적용 후에는 모두 "변경 안 함" 으로 돌아간다
    expect(screen.getByLabelText("촬영 감독")).toHaveValue("")
    expect(applyBlocked()).toBe(true)
  })

  it("T-ui-umeta-detail-14 [Business rule] cancel returns a field to unchanged; emptying a typed value also does", async () => {
    // Arrange
    const user = userEvent.setup()
    renderBulk(3, [F.kw, F.date])
    await user.click(screen.getByRole("button", { name: "납품일 값 지우기" }))
    expect(applyBlocked()).toBe(false)

    // Act + Assert: 취소
    await user.click(screen.getByRole("button", { name: "납품일 변경 취소" }))
    expect(screen.queryByText("값 지움")).not.toBeInTheDocument()
    expect(applyBlocked()).toBe(true)

    // Act + Assert: 입력했다가 비우면 변경 안 함
    const input = screen.getByLabelText("촬영 감독")
    await user.type(input, "가")
    await user.tab()
    expect(applyBlocked()).toBe(false)
    await user.clear(input)
    await user.tab()
    expect(applyBlocked()).toBe(true)
  })

  it("T-ui-umeta-detail-15 [Boundary] over 10,000 selected assets disables apply with a limit message; exactly 10,000 is allowed", async () => {
    // Arrange: 10,000 건 경계
    const user = userEvent.setup()
    const { unmount } = renderBulk(MAX_BULK_ASSETS, [F.kw])
    await user.click(screen.getByRole("button", { name: "촬영 감독 값 지우기" }))
    expect(applyBlocked()).toBe(false)
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
    unmount()

    // Act: 10,001 건
    renderBulk(MAX_BULK_ASSETS + 1, [F.kw])
    await user.click(screen.getByRole("button", { name: "촬영 감독 값 지우기" }))

    // Assert
    expect(applyBlocked()).toBe(true)
    expect(screen.getByRole("alert")).toHaveTextContent("한 번에 10,000개까지")
    expect(m.bulkSetAssetMetadata).not.toHaveBeenCalled()
  })

  it("T-ui-umeta-detail-16 [External] a bulk failure shows an inline error, no success toast, and keeps the edits", async () => {
    // Arrange
    const user = userEvent.setup()
    m.bulkSetAssetMetadata.mockRejectedValue({ code: "invalid_input", message: "bad" })
    renderBulk(3, [F.kw])
    await user.click(screen.getByRole("button", { name: "촬영 감독 값 지우기" }))

    // Act
    await user.click(apply())

    // Assert
    expect(await screen.findByRole("alert")).toHaveTextContent(/\S/)
    expect(toast.success).not.toHaveBeenCalled()
    expect(screen.getByText("값 지움")).toBeInTheDocument()
    expect(applyBlocked()).toBe(false)
  })

  it("T-ui-umeta-detail-17 [Regression] typing and clicking apply directly (no blur first) applies in one click; a blocked apply does nothing", async () => {
    // Arrange: 실화면에서 글 입력 직후 [적용] 을 누르면 첫 클릭이 사라졌다 (blur 로 변경이 확정되기 전엔 버튼이 disabled)
    const user = userEvent.setup()
    m.bulkSetAssetMetadata.mockResolvedValue(3)
    renderBulk(3, [F.kw])

    // Act + Assert: 변경이 없을 때의 클릭은 아무 것도 보내지 않는다 (jsdom 은 disabled 클릭 소실을 재현하지 못해 원인 속성도 단언)
    expect(apply()).not.toHaveAttribute("disabled")
    await user.click(apply())
    expect(m.bulkSetAssetMetadata).not.toHaveBeenCalled()

    // Act: 입력 후 포커스 이동 없이 바로 클릭
    await user.type(screen.getByLabelText("촬영 감독"), "김감독")
    await user.click(apply())

    // Assert
    await waitFor(() => expect(m.bulkSetAssetMetadata).toHaveBeenCalledTimes(1))
    expect(m.bulkSetAssetMetadata.mock.calls[0][1]).toEqual([{ op: "set", fieldId: 1, value: { kind: "keyword", value: "김감독" } }])
  })
})
