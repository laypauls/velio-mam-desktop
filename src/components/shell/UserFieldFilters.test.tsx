// Categories: Happy (T-ui-umeta-filter-01~07, 14) / Validation (T-ui-umeta-filter-08, 15) / Business rule (T-ui-umeta-filter-09~11, 14) / Boundary (T-ui-umeta-filter-14, 15)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { qk } from "@/hooks/keys"
import { useDeleteMetadataField, useDeleteMetadataOption, useMetadataFields } from "@/hooks/useMetadataFields"
import { useUserFieldFilterSync } from "@/hooks/useUserFieldFilterSync"
import type { MetadataField, MetadataFieldOption, MetadataValueType, UserFieldFilter } from "@/lib/ipc-contract-metadata"
import * as ipc from "@/lib/ipc-metadata"
import { pruneUserFields } from "@/lib/userFieldFilters"
import { DEFAULT_FILTERS, activeFilterCount, resetAllStores, useSearchStore } from "@/stores"
import { FilterPopover } from "./FilterPopover"

vi.mock("@/lib/ipc-metadata")
vi.mock("sonner", () => ({ toast: { info: vi.fn(), error: vi.fn(), success: vi.fn() } }))

const m = vi.mocked(ipc)
const opt = (optionId: number, label: string, position: number): MetadataFieldOption => ({ optionId, label, position })
function field(fieldId: number, name: string, valueType: MetadataValueType, options: MetadataFieldOption[] = []): MetadataField {
  return { fieldId, name, valueType, position: fieldId, options, createdAt: 0, updatedAt: 0 }
}

const PROJECT = field(1, "프로젝트", "enum", [opt(11, "봄 캠페인", 0), opt(12, "여름 시즌", 1), opt(13, "가을 특집", 2)])
const DIRECTOR = field(2, "촬영 감독", "keyword")
const PRICE = field(3, "단가", "number")
const DUE = field(4, "납품일", "date")
const DONE = field(5, "검수 완료", "boolean")
const USAGE = field(6, "용도", "multiEnum", [opt(21, "SNS", 0), opt(22, "TV", 1), opt(23, "옥외", 2)])
const ALL = [PROJECT, DIRECTOR, PRICE, DUE, DONE, USAGE]

let server: MetadataField[]
const userFields = () => useSearchStore.getState().filters.userFields

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  resetAllStores()
  server = ALL
  m.listMetadataFields.mockImplementation(() => Promise.resolve(structuredClone(server)))
  m.deleteMetadataField.mockImplementation((fieldId) => {
    server = server.filter((f) => f.fieldId !== fieldId)
    return Promise.resolve(3)
  })
  m.deleteMetadataOption.mockImplementation((optionId) => {
    server = server.map((f) => ({ ...f, options: f.options.filter((o) => o.optionId !== optionId) }))
    return Promise.resolve(3)
  })
})
afterEach(() => vi.clearAllMocks())

function makeClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } })
}

/** 필드 목록을 구독해 두는 관찰자 — 필드·선택지 삭제 훅의 무효화가 목록을 다시 받게 한다 */
function Observer() {
  useMetadataFields()
  return null
}
function Sync() {
  useUserFieldFilterSync()
  return null
}
function Deleter({ fieldId, optionId }: { fieldId?: number; optionId?: number }) {
  const delField = useDeleteMetadataField()
  const delOption = useDeleteMetadataOption()
  return (
    <>
      <button onClick={() => fieldId !== undefined && delField.mutate(fieldId)}>필드 삭제 실행</button>
      <button onClick={() => optionId !== undefined && delOption.mutate(optionId)}>선택지 삭제 실행</button>
    </>
  )
}

async function openPopover(qc = makeClient()) {
  const user = userEvent.setup()
  render(
    <QueryClientProvider client={qc}>
      <FilterPopover />
    </QueryClientProvider>
  )
  await user.click(screen.getByRole("button", { name: /^필터/ }))
  await user.click(await screen.findByRole("button", { name: /^사용자 메타데이터/ }))
  return user
}

/** 필드 접이식을 열고 그 필드의 입력 묶음(group)을 돌려준다 */
async function openField(user: ReturnType<typeof userEvent.setup>, name: string) {
  await user.click(await screen.findByRole("button", { name: new RegExp(`^${name}`) }))
  return within(screen.getByRole("group", { name }))
}

describe("UserFieldFilters conditions", () => {
  it("T-ui-umeta-filter-01 [Happy] keyword: values are trimmed, de-duplicated case-insensitively, and sent as keywordAnyOf", async () => {
    // Arrange
    const user = await openPopover()
    const g = await openField(user, "촬영 감독")
    const input = g.getByRole("textbox", { name: "촬영 감독 찾을 값" })
    // Act
    await user.type(input, "  Kim  {Enter}")
    await user.type(input, "kim{Enter}")
    await user.type(input, "이영희{Enter}")
    // Assert
    expect(userFields()).toEqual([{ kind: "keywordAnyOf", fieldId: 2, values: ["Kim", "이영희"] }])
    // Act: 값 하나 제거 → 나머지만
    await user.click(g.getByRole("button", { name: "Kim 제거" }))
    expect(userFields()).toEqual([{ kind: "keywordAnyOf", fieldId: 2, values: ["이영희"] }])
    // Act: 마지막 값 제거 → 조건 자체가 사라지고 필터는 기본값과 같은 JSON
    await user.click(g.getByRole("button", { name: "이영희 제거" }))
    expect(userFields()).toBeUndefined()
    expect(JSON.stringify(useSearchStore.getState().filters)).toBe(JSON.stringify(DEFAULT_FILTERS))
  })

  it("T-ui-umeta-filter-02 [Boundary] keyword: blank input adds nothing, typed text is added on blur", async () => {
    // Arrange
    const user = await openPopover()
    const g = await openField(user, "촬영 감독")
    const input = g.getByRole("textbox", { name: "촬영 감독 찾을 값" })
    // Act
    await user.type(input, "   {Enter}")
    // Assert
    expect(userFields()).toBeUndefined()
    // Act: Enter 없이 입력만 하고 다른 곳으로 이동
    await user.type(input, "박")
    await user.tab()
    // Assert
    expect(userFields()).toEqual([{ kind: "keywordAnyOf", fieldId: 2, values: ["박"] }])
  })

  it("T-ui-umeta-filter-03 [Happy] number: min/max map to numberRange; one side only keeps the other null", async () => {
    // Arrange
    const user = await openPopover()
    const g = await openField(user, "단가")
    // Act
    fireEvent.change(g.getByRole("spinbutton", { name: "단가 최솟값" }), { target: { value: "1.5" } })
    // Assert
    expect(userFields()).toEqual([{ kind: "numberRange", fieldId: 3, min: 1.5, max: null }])
    // Act
    fireEvent.change(g.getByRole("spinbutton", { name: "단가 최댓값" }), { target: { value: "-20" } })
    fireEvent.change(g.getByRole("spinbutton", { name: "단가 최솟값" }), { target: { value: "-30" } })
    // Assert: 음수·같은 값 경계
    expect(userFields()).toEqual([{ kind: "numberRange", fieldId: 3, min: -30, max: -20 }])
    // Act: 둘 다 비우면 조건 제거
    fireEvent.change(g.getByRole("spinbutton", { name: "단가 최솟값" }), { target: { value: "" } })
    fireEvent.change(g.getByRole("spinbutton", { name: "단가 최댓값" }), { target: { value: "" } })
    expect(userFields()).toBeUndefined()
  })

  it("T-ui-umeta-filter-04 [Happy] date: from/to map to dateRange as YYYYMMDD integers", async () => {
    // Arrange
    const user = await openPopover()
    const g = await openField(user, "납품일")
    // Act
    fireEvent.change(g.getByLabelText("납품일 시작일"), { target: { value: "2026-11-01" } })
    fireEvent.change(g.getByLabelText("납품일 종료일"), { target: { value: "2026-11-30" } })
    // Assert
    expect(userFields()).toEqual([{ kind: "dateRange", fieldId: 4, min: 20261101, max: 20261130 }])
    // Act: 종료일만 비우면 한쪽 열린 구간
    fireEvent.change(g.getByLabelText("납품일 종료일"), { target: { value: "" } })
    expect(userFields()).toEqual([{ kind: "dateRange", fieldId: 4, min: 20261101, max: null }])
  })

  it("T-ui-umeta-filter-05 [Happy] boolean: 3-state radio sends bool true/false and removes the condition on 전체", async () => {
    // Arrange
    const user = await openPopover()
    const g = await openField(user, "검수 완료")
    const group = within(g.getByRole("radiogroup", { name: "검수 완료 값" }))
    // Act / Assert
    await user.click(group.getByRole("radio", { name: "예" }))
    expect(userFields()).toEqual([{ kind: "bool", fieldId: 5, value: true }])
    await user.click(group.getByRole("radio", { name: "아니요" }))
    expect(userFields()).toEqual([{ kind: "bool", fieldId: 5, value: false }])
    await user.click(group.getByRole("radio", { name: "전체" }))
    expect(userFields()).toBeUndefined()
  })

  it("T-ui-umeta-filter-06 [Happy] enum: checklist sends enumAnyOf with option ids in option order", async () => {
    // Arrange
    const user = await openPopover()
    const g = await openField(user, "프로젝트")
    // Act: 뒤 선택지를 먼저 체크
    await user.click(g.getByRole("checkbox", { name: "가을 특집" }))
    await user.click(g.getByRole("checkbox", { name: "봄 캠페인" }))
    // Assert
    expect(userFields()).toEqual([{ kind: "enumAnyOf", fieldId: 1, optionIds: [11, 13] }])
    // Act: 모두 해제 → 빈 목록을 보내지 않고 조건 제거
    await user.click(g.getByRole("checkbox", { name: "가을 특집" }))
    await user.click(g.getByRole("checkbox", { name: "봄 캠페인" }))
    expect(userFields()).toBeUndefined()
  })

  it("T-ui-umeta-filter-07 [Happy] multiEnum: checklist + any/all toggle sends multiEnum with the all flag", async () => {
    // Arrange
    const user = await openPopover()
    const g = await openField(user, "용도")
    // Act
    await user.click(g.getByRole("checkbox", { name: "TV" }))
    await user.click(g.getByRole("checkbox", { name: "SNS" }))
    // Assert: 기본은 하나라도 포함
    expect(userFields()).toEqual([{ kind: "multiEnum", fieldId: 6, optionIds: [21, 22], all: false }])
    // Act
    await user.click(g.getByRole("radio", { name: "모두 포함" }))
    // Assert
    expect(userFields()).toEqual([{ kind: "multiEnum", fieldId: 6, optionIds: [21, 22], all: true }])
    // Act: 토글만 바꾼 상태에서 선택을 모두 비우면 조건 제거
    await user.click(g.getByRole("checkbox", { name: "SNS" }))
    await user.click(g.getByRole("checkbox", { name: "TV" }))
    expect(userFields()).toBeUndefined()
  })

  it("T-ui-umeta-filter-08 [Happy] every type: 값 있음 / 값 없음 send has / missing and disable the value inputs", async () => {
    // Arrange
    const user = await openPopover()
    const cases: [string, number][] = [["촬영 감독", 2], ["단가", 3], ["납품일", 4], ["검수 완료", 5], ["프로젝트", 1], ["용도", 6]]
    for (const [name, fieldId] of cases) {
      const g = await openField(user, name)
      const presence = within(g.getByRole("radiogroup", { name: `${name} 값 유무` }))
      // Act / Assert
      await user.click(presence.getByRole("radio", { name: "값 있음" }))
      expect(userFields()).toContainEqual({ kind: "has", fieldId })
      await user.click(presence.getByRole("radio", { name: "값 없음" }))
      expect(userFields()).toContainEqual({ kind: "missing", fieldId })
      expect(userFields()?.filter((c) => c.fieldId === fieldId)).toHaveLength(1) // 필드당 조건 1개
    }
    expect(userFields()).toHaveLength(6)
    // 값 없음이 켜진 동안 값 입력은 비활성
    expect(screen.getByRole("textbox", { name: "촬영 감독 찾을 값" })).toBeDisabled()
    // Act: 전체로 되돌리면 조건 제거
    await user.click(within(screen.getByRole("radiogroup", { name: "촬영 감독 값 유무" })).getByRole("radio", { name: "전체" }))
    expect(userFields()?.some((c) => c.fieldId === 2)).toBe(false)
    expect(screen.getByRole("textbox", { name: "촬영 감독 찾을 값" })).toBeEnabled()
  })

  it("T-ui-umeta-filter-09 [Validation] min > max shows an inline aria-invalid error and is not sent; fixing it sends (number and date)", async () => {
    // Arrange
    const user = await openPopover()
    const num = await openField(user, "단가")
    fireEvent.change(num.getByRole("spinbutton", { name: "단가 최솟값" }), { target: { value: "5" } })
    fireEvent.change(num.getByRole("spinbutton", { name: "단가 최댓값" }), { target: { value: "9" } })
    expect(userFields()).toEqual([{ kind: "numberRange", fieldId: 3, min: 5, max: 9 }])
    // Act: 최댓값을 최솟값보다 작게
    fireEvent.change(num.getByRole("spinbutton", { name: "단가 최댓값" }), { target: { value: "3" } })
    // Assert: 오류 표시, 마지막 유효 조건 유지(역전된 3 은 전송 안 됨)
    expect(num.getByRole("alert")).toHaveTextContent("최소값이 최대값보다 클 수 없습니다.")
    expect(num.getByRole("spinbutton", { name: "단가 최댓값" })).toHaveAttribute("aria-invalid", "true")
    expect(num.getByRole("spinbutton", { name: "단가 최솟값" })).toHaveAccessibleDescription("최소값이 최대값보다 클 수 없습니다.")
    expect(userFields()).toEqual([{ kind: "numberRange", fieldId: 3, min: 5, max: 9 }])
    // Act: 바로잡기
    fireEvent.change(num.getByRole("spinbutton", { name: "단가 최댓값" }), { target: { value: "30" } })
    expect(num.queryByRole("alert")).not.toBeInTheDocument()
    expect(userFields()).toEqual([{ kind: "numberRange", fieldId: 3, min: 5, max: 30 }])
    // 날짜
    const date = await openField(user, "납품일")
    fireEvent.change(date.getByLabelText("납품일 종료일"), { target: { value: "2026-10-01" } })
    fireEvent.change(date.getByLabelText("납품일 시작일"), { target: { value: "2026-10-05" } })
    expect(date.getByRole("alert")).toBeInTheDocument()
    expect(userFields()?.some((c) => c.kind === "dateRange" && c.min !== null && c.max !== null)).toBe(false)
    // 같은 날은 유효
    fireEvent.change(date.getByLabelText("납품일 시작일"), { target: { value: "2026-10-01" } })
    expect(date.queryByRole("alert")).not.toBeInTheDocument()
    expect(userFields()).toContainEqual({ kind: "dateRange", fieldId: 4, min: 20261001, max: 20261001 })
  })
})

describe("UserFieldFilters state", () => {
  it("T-ui-umeta-filter-10 [Business rule] 초기화 clears user conditions and the inputs; the filter badge counts one per field", async () => {
    // Arrange
    const user = await openPopover()
    const g = await openField(user, "프로젝트")
    await user.click(g.getByRole("checkbox", { name: "봄 캠페인" }))
    const d = await openField(user, "촬영 감독")
    await user.type(d.getByRole("textbox", { name: "촬영 감독 찾을 값" }), "Kim{Enter}")
    // Assert: 필터 버튼 배지 = 2 (필드당 1), 섹션 배지도 2
    expect(activeFilterCount(useSearchStore.getState().filters)).toBe(2)
    expect(screen.getByLabelText("적용된 필터 2개")).toHaveTextContent("2")
    // Act
    await user.click(screen.getByRole("button", { name: "필터 초기화" }))
    // Assert
    expect(userFields()).toBeUndefined()
    expect(JSON.stringify(useSearchStore.getState().filters)).toBe(JSON.stringify(DEFAULT_FILTERS))
    expect(screen.queryByLabelText(/^적용된 필터/)).not.toBeInTheDocument()
    // 초기화 뒤 섹션을 다시 열어도 입력은 비어 있다
    await user.click(await screen.findByRole("button", { name: /^사용자 메타데이터/ }))
    const again = await openField(user, "프로젝트")
    expect(again.getByRole("checkbox", { name: "봄 캠페인" })).not.toBeChecked()
  })

  it("T-ui-umeta-filter-11 [Business rule] reopening the popover restores each field's inputs from the active conditions", async () => {
    // Arrange: 이미 걸린 필터
    useSearchStore.getState().patchFilters({
      userFields: [{ kind: "dateRange", fieldId: 4, min: 20261101, max: null }, { kind: "missing", fieldId: 2 }],
    })
    // Act: 조건이 있으면 섹션이 처음부터 열려 있으므로 섹션 토글은 누르지 않는다
    const user = userEvent.setup()
    render(
      <QueryClientProvider client={makeClient()}>
        <FilterPopover />
      </QueryClientProvider>
    )
    await user.click(screen.getByRole("button", { name: "필터" }))
    // Assert: 조건이 있는 필드는 열려 있고 값이 채워진다
    const due = within(await screen.findByRole("group", { name: "납품일" }))
    expect(due.getByLabelText("납품일 시작일")).toHaveValue("2026-11-01")
    const dir = within(screen.getByRole("group", { name: "촬영 감독" }))
    expect(dir.getByRole("radio", { name: "값 없음" })).toBeChecked()
  })

  it("T-ui-umeta-filter-14 [Business rule] stale conditions set before the field list arrives are cleaned once the list is received", async () => {
    // Arrange: 필드 77 은 존재하지 않는다
    useSearchStore.getState().patchFilters({ userFields: [{ kind: "has", fieldId: 77 }, { kind: "has", fieldId: 2 }] })
    // Act
    render(
      <QueryClientProvider client={makeClient()}>
        <Sync />
      </QueryClientProvider>
    )
    // Assert
    await waitFor(() => expect(userFields()).toEqual([{ kind: "has", fieldId: 2 }]))
    expect(toast.info).toHaveBeenCalledTimes(1)
  })

  it("T-ui-umeta-filter-15 [Business rule] deleting a field (or option) while its condition is active removes it from the filters, so search never sends a stale id", async () => {
    // Arrange
    useSearchStore.getState().patchFilters({
      userFields: [{ kind: "has", fieldId: 5 }, { kind: "enumAnyOf", fieldId: 1, optionIds: [11, 12] }, { kind: "missing", fieldId: 2 }],
    })
    const user = userEvent.setup()
    render(
      <QueryClientProvider client={makeClient()}>
        <Observer />
        <Deleter fieldId={5} optionId={12} />
      </QueryClientProvider>
    )
    await waitFor(() => expect(m.listMetadataFields).toHaveBeenCalled())
    // Act: 필드 삭제
    await user.click(screen.getByRole("button", { name: "필드 삭제 실행" }))
    // Assert
    await waitFor(() => expect(userFields()).toEqual([{ kind: "enumAnyOf", fieldId: 1, optionIds: [11, 12] }, { kind: "missing", fieldId: 2 }]))
    expect(toast.info).toHaveBeenLastCalledWith("삭제된 필드나 선택지를 쓰는 필터 조건 1개를 제외했습니다.")
    // Act: 선택지 삭제 (남은 선택지로 조건 유지)
    await user.click(screen.getByRole("button", { name: "선택지 삭제 실행" }))
    // Assert
    await waitFor(() => expect(userFields()).toEqual([{ kind: "enumAnyOf", fieldId: 1, optionIds: [11] }, { kind: "missing", fieldId: 2 }]))
  })

  it("T-ui-umeta-filter-16 [Business rule] the always-mounted sync drops a condition whose field disappeared from a refetched list", async () => {
    // Arrange
    useSearchStore.getState().patchFilters({ userFields: [{ kind: "bool", fieldId: 5, value: true }] })
    const qc = makeClient()
    render(
      <QueryClientProvider client={qc}>
        <Sync />
      </QueryClientProvider>
    )
    await waitFor(() => expect(m.listMetadataFields).toHaveBeenCalledTimes(1))
    expect(userFields()).toHaveLength(1) // 목록에 있으면 유지
    // Act: 다른 경로에서 필드가 삭제된 뒤 목록 갱신
    server = ALL.filter((f) => f.fieldId !== 5)
    await act(() => qc.invalidateQueries({ queryKey: qk.metadataFields }))
    // Assert
    await waitFor(() => expect(userFields()).toBeUndefined())
  })

  it("T-ui-umeta-filter-17 [External] a failed field list keeps the active conditions (nothing is dropped on load failure)", async () => {
    // Arrange
    m.listMetadataFields.mockRejectedValue(new Error("boom"))
    useSearchStore.getState().patchFilters({ userFields: [{ kind: "has", fieldId: 5 }] })
    // Act
    render(
      <QueryClientProvider client={makeClient()}>
        <Sync />
      </QueryClientProvider>
    )
    await waitFor(() => expect(m.listMetadataFields).toHaveBeenCalled())
    // Assert
    expect(userFields()).toEqual([{ kind: "has", fieldId: 5 }])
    expect(toast.info).not.toHaveBeenCalled()
  })
})

describe("UserFieldFilters rendering", () => {
  it("T-ui-umeta-filter-18 [Boundary] no fields defined: the section is not rendered", async () => {
    // Arrange
    server = []
    const user = userEvent.setup()
    render(
      <QueryClientProvider client={makeClient()}>
        <FilterPopover />
      </QueryClientProvider>
    )
    // Act
    await user.click(screen.getByRole("button", { name: /^필터/ }))
    await waitFor(() => expect(m.listMetadataFields).toHaveBeenCalled())
    // Assert
    expect(screen.getByText("미디어 유형")).toBeInTheDocument()
    expect(screen.queryByText("사용자 메타데이터")).not.toBeInTheDocument()
  })

  it("T-ui-umeta-filter-19 [Boundary] 20 fields including a 100-character name all render, and the long name's inputs are labelled with it", async () => {
    // Arrange
    const longName = "아주 긴 사용자 정의 필드 이름 ".repeat(6).trim().slice(0, 100)
    server = Array.from({ length: 20 }, (_, i) =>
      i === 7 ? field(8, longName, "keyword") : field(i + 1, `필드 ${i + 1}`, i % 2 === 0 ? "number" : "keyword")
    )
    const user = await openPopover()
    // Assert: 20개 모두 나열
    const list = await screen.findByRole("list", { name: "사용자 메타데이터" })
    expect(within(list).getAllByRole("listitem")).toHaveLength(20)
    expect(within(list).getByText(longName)).toBeInTheDocument()
    expect(within(list).getByText("필드 20")).toBeInTheDocument()
    // Act: 긴 이름 필드를 열어 입력
    const g = await openField(user, longName)
    await user.type(g.getByRole("textbox", { name: `${longName} 찾을 값` }), "x{Enter}")
    // Assert
    expect(userFields()).toEqual([{ kind: "keywordAnyOf", fieldId: 8, values: ["x"] }])
  })

  it("T-ui-umeta-filter-21 [Regression] the popover is bounded by the available viewport height and scrolls itself (small windows hid the reset button)", async () => {
    // Arrange: 실화면에서 필드 20개를 펼치면 팝오버가 720px 창 아래로 넘쳐 [필터 초기화] 에 닿을 수 없었다
    server = Array.from({ length: 20 }, (_, i) => field(i + 1, `필드 ${i + 1}`, "keyword"))
    await openPopover()
    // Assert: jsdom 은 레이아웃이 없어 실제 높이는 tauri dev 로 확인했고, 여기서는 원인 클래스(가용 높이 상한 + 자체 스크롤)를 고정한다
    const content = document.querySelector("[data-slot=popover-content]")
    expect(content).not.toBeNull()
    expect(content?.className).toContain("max-h-(--radix-popover-content-available-height)")
    expect(content?.className).toContain("overflow-y-auto")
  })
})

describe("pruneUserFields", () => {
  it("T-ui-umeta-filter-20 [Boundary] returns the same array when nothing is stale; drops type mismatches and fully-removed option lists", () => {
    // Arrange
    const ok: UserFieldFilter[] = [{ kind: "has", fieldId: 1 }, { kind: "enumAnyOf", fieldId: 1, optionIds: [11] }]
    // Act / Assert: 변경 없음 → 같은 참조
    const same = pruneUserFields(ok, ALL)
    expect(same.kept).toBe(ok)
    expect(same.dropped).toBe(0)
    // 타입 불일치(필드 2 는 keyword 인데 bool 조건), 선택지 전부 없음
    const stale: UserFieldFilter[] = [{ kind: "bool", fieldId: 2, value: true }, { kind: "multiEnum", fieldId: 6, optionIds: [1, 2], all: false }]
    expect(pruneUserFields(stale, ALL)).toEqual({ kept: [], dropped: 2 })
  })
})
