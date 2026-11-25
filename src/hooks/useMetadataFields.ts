import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import {
  addMetadataOption,
  createMetadataField,
  deleteMetadataField,
  deleteMetadataOption,
  listMetadataFields,
  metadataFieldValueCount,
  metadataOptionUsage,
  renameMetadataField,
  renameMetadataOption,
  reorderMetadataFields,
  reorderMetadataOptions,
} from "@/lib/ipc-metadata"
import type { MetadataField, MetadataValueType } from "@/lib/ipc-contract-metadata"
import { dropStaleUserFilters } from "@/lib/dropStaleUserFilters"
import { qk } from "./keys"

/**
 * 사용자 정의 메타데이터 필드 목록(표시 순서, 선택지 포함). 상세 패널·갤러리 필터·설정이 함께 쓴다.
 * 필드 변경 알림 = `qk.metadataFields` 무효화 — 이 훅을 쓰는 화면은 설정에서 필드를 바꾸면 자동으로 다시 받는다.
 */
export function useMetadataFields() {
  return useQuery({ queryKey: qk.metadataFields, queryFn: listMetadataFields })
}

/** 필드 정의가 바뀐 뒤 갱신할 캐시. `valuesChanged` = 값이 지워질 수 있는 변경(필드·선택지 삭제) → 검색 결과·상세도 다시 받는다. */
export function useMetadataFieldsChanged() {
  const qc = useQueryClient()
  return async (valuesChanged = false) => {
    await qc.invalidateQueries({ queryKey: qk.metadataFields })
    if (!valuesChanged) return
    // 검색을 다시 받기 전에 사라진 필드·선택지를 쓰는 필터 조건부터 뺀다 — 낡은 조건으로 요청하면 invalid_input
    const fields = qc.getQueryData<MetadataField[]>(qk.metadataFields)
    if (fields) dropStaleUserFilters(fields)
    await Promise.all([qc.invalidateQueries({ queryKey: qk.search }), qc.invalidateQueries({ queryKey: qk.assets })])
  }
}

export function useCreateMetadataField() {
  const changed = useMetadataFieldsChanged()
  return useMutation({
    mutationFn: (v: { name: string; valueType: MetadataValueType; options: string[] }) => createMetadataField(v.name, v.valueType, v.options),
    onSuccess: () => changed(),
  })
}

export function useRenameMetadataField() {
  const changed = useMetadataFieldsChanged()
  return useMutation({
    mutationFn: (v: { fieldId: number; name: string }) => renameMetadataField(v.fieldId, v.name),
    onSuccess: () => changed(),
  })
}

/** 필드 순서 변경 — 캐시를 먼저 새 순서로 바꿔 즉시 반영하고, 실패하면 서버 상태로 되돌린다. */
export function useReorderMetadataFields() {
  const qc = useQueryClient()
  const changed = useMetadataFieldsChanged()
  return useMutation({
    mutationFn: (fieldIds: number[]) => reorderMetadataFields(fieldIds),
    onMutate: (fieldIds) => {
      qc.setQueryData<MetadataField[]>(qk.metadataFields, (prev) => {
        if (!prev) return prev
        const byId = new Map(prev.map((f) => [f.fieldId, f]))
        return fieldIds.flatMap((id) => byId.get(id) ?? [])
      })
    },
    onSettled: () => changed(),
  })
}

/** 필드 삭제 — 목록에서 즉시 사라지고 값 정리는 백그라운드. 값이 지워지므로 검색·상세도 갱신한다. */
export function useDeleteMetadataField() {
  const changed = useMetadataFieldsChanged()
  return useMutation({
    mutationFn: (fieldId: number) => deleteMetadataField(fieldId),
    onSuccess: () => changed(true),
  })
}

export function useAddMetadataOption() {
  const changed = useMetadataFieldsChanged()
  return useMutation({
    mutationFn: (v: { fieldId: number; label: string }) => addMetadataOption(v.fieldId, v.label),
    onSuccess: () => changed(),
  })
}

export function useRenameMetadataOption() {
  const changed = useMetadataFieldsChanged()
  return useMutation({
    mutationFn: (v: { optionId: number; label: string }) => renameMetadataOption(v.optionId, v.label),
    onSuccess: () => changed(),
  })
}

export function useReorderMetadataOptions() {
  const changed = useMetadataFieldsChanged()
  return useMutation({
    mutationFn: (v: { fieldId: number; optionIds: number[] }) => reorderMetadataOptions(v.fieldId, v.optionIds),
    onSettled: () => changed(),
  })
}

export function useDeleteMetadataOption() {
  const changed = useMetadataFieldsChanged()
  return useMutation({
    mutationFn: (optionId: number) => deleteMetadataOption(optionId),
    onSuccess: () => changed(true),
  })
}

/** 삭제 확인창용 — 이 필드에 값을 가진 자산 수 (열 때마다 새로 조회) */
export function useMetadataFieldValueCount(fieldId: number | null) {
  return useQuery({
    queryKey: qk.metadataFieldValueCount(fieldId ?? -1),
    queryFn: () => metadataFieldValueCount(fieldId as number),
    enabled: fieldId !== null,
    gcTime: 0,
    retry: false,
  })
}

/** 삭제 확인창용 — 이 선택지를 값으로 가진 자산 수 (열 때마다 새로 조회) */
export function useMetadataOptionUsage(optionId: number | null) {
  return useQuery({
    queryKey: qk.metadataOptionUsage(optionId ?? -1),
    queryFn: () => metadataOptionUsage(optionId as number),
    enabled: optionId !== null,
    gcTime: 0,
    retry: false,
  })
}
