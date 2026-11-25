// Categories: Happy (T-ui-ipc-umeta-01 정의, 02 값) / Boundary (03 userFields 직렬화) / External (04 오류 매핑)
import { invoke } from "@tauri-apps/api/core"
import { beforeEach, describe, expect, it, vi } from "vitest"
import type { MetadataField, MetadataPatch, SearchFilters, UserFieldFilter } from "./ipc-contract"
import { errorKey, isCommandError } from "./errors"
import {
  addMetadataOption,
  bulkSetAssetMetadata,
  createMetadataField,
  deleteMetadataField,
  deleteMetadataOption,
  getAssetMetadata,
  listMetadataFields,
  metadataFieldValueCount,
  metadataOptionUsage,
  renameMetadataField,
  renameMetadataOption,
  reorderMetadataFields,
  reorderMetadataOptions,
  setAssetMetadata,
} from "./ipc"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))

/** Rust `MetadataFieldView` 가 돌려주는 모양 (camelCase, valueType 은 camelCase) */
const FIELD: MetadataField = {
  fieldId: 3,
  name: "프로젝트",
  valueType: "multiEnum",
  position: 0,
  options: [{ optionId: 11, label: "봄", position: 0 }],
  createdAt: 1_700_000_000_000,
  updatedAt: 1_700_000_000_000,
}

beforeEach(() => {
  vi.mocked(invoke).mockReset()
})

describe("ipc metadata definition commands", () => {
  it("T-ui-ipc-umeta-01 [Happy] definition wrappers use the Rust command and argument names and return the server shapes", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce([FIELD]) // list
    vi.mocked(invoke).mockResolvedValueOnce(FIELD) // create
    vi.mocked(invoke).mockResolvedValueOnce(FIELD) // rename
    vi.mocked(invoke).mockResolvedValueOnce(undefined) // reorder
    vi.mocked(invoke).mockResolvedValueOnce(5) // delete
    vi.mocked(invoke).mockResolvedValueOnce(5) // count
    vi.mocked(invoke).mockResolvedValueOnce(FIELD.options[0]) // add option
    vi.mocked(invoke).mockResolvedValueOnce(FIELD.options[0]) // rename option
    vi.mocked(invoke).mockResolvedValueOnce(undefined) // reorder options
    vi.mocked(invoke).mockResolvedValueOnce(2) // delete option
    vi.mocked(invoke).mockResolvedValueOnce(2) // usage
    // Act
    const listed = await listMetadataFields()
    await createMetadataField("프로젝트", "multiEnum", ["봄"])
    await renameMetadataField(3, "작업")
    await reorderMetadataFields([3, 1])
    const deleted = await deleteMetadataField(3)
    const count = await metadataFieldValueCount(3)
    await addMetadataOption(3, "여름")
    await renameMetadataOption(11, "겨울")
    await reorderMetadataOptions(3, [12, 11])
    const optDeleted = await deleteMetadataOption(11)
    const usage = await metadataOptionUsage(11)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["list_metadata_fields"],
      ["create_metadata_field", { name: "프로젝트", valueType: "multiEnum", options: ["봄"] }],
      ["rename_metadata_field", { fieldId: 3, name: "작업" }],
      ["reorder_metadata_fields", { fieldIds: [3, 1] }],
      ["delete_metadata_field", { fieldId: 3 }],
      ["metadata_field_value_count", { fieldId: 3 }],
      ["add_metadata_option", { fieldId: 3, label: "여름" }],
      ["rename_metadata_option", { optionId: 11, label: "겨울" }],
      ["reorder_metadata_options", { fieldId: 3, optionIds: [12, 11] }],
      ["delete_metadata_option", { optionId: 11 }],
      ["metadata_option_usage", { optionId: 11 }],
    ])
    expect(listed).toEqual([FIELD])
    expect([deleted, count, optDeleted, usage]).toEqual([5, 5, 2, 2])
  })
})

describe("ipc metadata value commands", () => {
  it("T-ui-ipc-umeta-02 [Happy] value wrappers send tagged values and set/clear patches and return counts", async () => {
    // Arrange
    const patches: MetadataPatch[] = [
      { op: "set", fieldId: 1, value: { kind: "keyword", value: "김" } },
      { op: "set", fieldId: 2, value: { kind: "date", value: 20261120 } },
      { op: "set", fieldId: 3, value: { kind: "multiEnum", value: [11, 12] } },
      { op: "clear", fieldId: 4 },
    ]
    const stored = [{ fieldId: 1, value: { kind: "keyword", value: "김" } }]
    vi.mocked(invoke).mockResolvedValueOnce(stored)
    vi.mocked(invoke).mockResolvedValueOnce(undefined)
    vi.mocked(invoke).mockResolvedValueOnce(1234)
    // Act
    const got = await getAssetMetadata(7)
    await setAssetMetadata(7, patches)
    const changed = await bulkSetAssetMetadata([7, 8], patches)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["get_asset_metadata", { assetId: 7 }],
      ["set_asset_metadata", { assetId: 7, patches }],
      ["bulk_set_asset_metadata", { assetIds: [7, 8], patches }],
    ])
    expect(got).toEqual(stored)
    expect(changed).toBe(1234)
  })
})

describe("SearchFilters.userFields", () => {
  it("T-ui-ipc-umeta-03 [Boundary] userFields is optional and every condition kind matches the Rust serde shape", () => {
    // Arrange: 기존 코드처럼 userFields 없이 만든 필터가 컴파일되고, 모든 조건 종류가 계약 JSON 과 같다
    const base: SearchFilters = {
      mediaTypes: [], includeOffline: true, includeMissing: false, capturedFrom: null, capturedTo: null, sizeMin: null, sizeMax: null,
      durationMinMs: null, durationMaxMs: null, folder: null, ratingMin: null, ratingMax: null, favoritesOnly: false, collectionId: null, tagIds: [],
    }
    const conds: UserFieldFilter[] = [
      { kind: "keywordAnyOf", fieldId: 1, values: ["alpha"] },
      { kind: "numberRange", fieldId: 2, min: 1.5, max: null },
      { kind: "dateRange", fieldId: 3, min: 20261101, max: 20261130 },
      { kind: "bool", fieldId: 4, value: true },
      { kind: "enumAnyOf", fieldId: 5, optionIds: [1, 2] },
      { kind: "multiEnum", fieldId: 6, optionIds: [3, 4], all: true },
      { kind: "has", fieldId: 1 },
      { kind: "missing", fieldId: 1 },
    ]
    // Act
    const json = JSON.parse(JSON.stringify({ ...base, userFields: conds }))
    // Assert
    expect(base.userFields).toBeUndefined()
    expect(json.userFields).toEqual(conds)
    expect(json.userFields.map((c: UserFieldFilter) => c.kind)).toEqual([
      "keywordAnyOf", "numberRange", "dateRange", "bool", "enumAnyOf", "multiEnum", "has", "missing",
    ])
  })
})

describe("ipc metadata errors", () => {
  it("T-ui-ipc-umeta-04 [External] structured command errors pass through and map to known message keys", async () => {
    // Arrange + Act + Assert: 중복 이름 / 없는 필드 / 20개 초과·자산 1만 건 초과
    vi.mocked(invoke).mockRejectedValueOnce({ code: "already_registered", message: "folder is already registered" })
    const dup = await createMetadataField("프로젝트", "keyword", []).catch((e: unknown) => e)
    vi.mocked(invoke).mockRejectedValueOnce({ code: "not_found", message: "not found" })
    const missing = await renameMetadataField(99, "x").catch((e: unknown) => e)
    vi.mocked(invoke).mockRejectedValueOnce({ code: "invalid_input", message: "invalid input: assets" })
    const tooMany = await bulkSetAssetMetadata(Array.from({ length: 10_001 }, (_, i) => i + 1), []).catch((e: unknown) => e)

    expect(dup).toEqual({ code: "already_registered", message: "folder is already registered" })
    expect(missing).toMatchObject({ code: "not_found" })
    expect(isCommandError(tooMany)).toBe(true)
    expect(errorKey(tooMany)).toBe("errors.invalid_input")
  })
})
