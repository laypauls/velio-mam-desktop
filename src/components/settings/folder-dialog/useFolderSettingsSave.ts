import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { MEDIA_TYPE_ORDER, noMediaTypeSelected } from "@/components/folders/MediaTypeFields"
import { useSetFolderAlias, useSetFolderMediaTypes } from "@/hooks/useFolderAdmin"
import { useSetFolderRescanPolicy, useSetFolderWatchMode } from "@/hooks/useFolderDetection"
import { errorKey } from "@/lib/errors"
import type { ChangeDetectorInfo, ManagedFolder, MediaTypeChangeMode, MediaTypeSet, WatchOptions } from "@/lib/ipc-contract"
import { draftFromPolicy, policyFromDraft, sameDraft, validateDraft, type PolicyDraft } from "@/lib/rescan-policy"
import { WATCH_AUTO } from "../detection/WatchModeSection"

/** 백엔드 검증과 같은 상한 (문자 수 기준). */
export const FOLDER_ALIAS_MAX = 64

/** 저장 순서 — 별칭 → 스캔 대상 → 재스캔 정책 → 감지 방식. */
export type SaveAxis = "alias" | "types" | "policy" | "watch"

export interface WatchDraft {
  mode: string
  /** 감지기별 옵션 입력 중 문자열 (key → 값) — 감지기를 바꾸면 비운다 */
  options: Record<string, string>
  /** 공통 옵션 `exclusiveOpenCheck` (복사 완료 확인에 배타적 열기 사용, 기본 켜짐) — 감지기를 바꿔도 유지한다 */
  exclusive: boolean
}

/** 공통 옵션 키 — 감지기 선언 목록에 없고 모든 감지기·자동 모드가 받는다 */
const EXCLUSIVE_KEY = "exclusiveOpenCheck"

const watchDraftOf = (f: ManagedFolder): WatchDraft => ({
  mode: f.watchMode ?? WATCH_AUTO,
  options: Object.fromEntries(
    Object.entries(f.watchOptions ?? {})
      .filter(([k]) => k !== EXCLUSIVE_KEY)
      .map(([k, v]) => [k, String(v)]),
  ),
  exclusive: f.watchOptions?.[EXCLUSIVE_KEY] !== false,
})

/**
 * 폴더 설정 다이얼로그의 편집 초안 + 순차 저장.
 * 백엔드에 한 번에 저장하는 IPC 가 없어 바뀐 축만 순서대로 호출한다 — 성공한 축은 기준값을 갱신해
 * 부분 실패 뒤 재시도가 같은 호출을 되풀이하지 않는다.
 */
export function useFolderSettingsSave(folder: ManagedFolder, detectors: ChangeDetectorInfo[] | undefined, defaultHours: number) {
  const { t } = useTranslation()
  const setAlias = useSetFolderAlias()
  const setTypes = useSetFolderMediaTypes()
  const setPolicy = useSetFolderRescanPolicy()
  const setMode = useSetFolderWatchMode()

  // 기준(저장된 상태)과 초안
  const [aliasBase, setAliasBase] = useState(folder.alias ?? "")
  const [alias, setAliasDraft] = useState(aliasBase)
  const [typesBase, setTypesBase] = useState<MediaTypeSet>(folder.mediaTypes)
  const [types, setTypesDraft] = useState<MediaTypeSet>(folder.mediaTypes)
  const [typeMode, setTypeMode] = useState<MediaTypeChangeMode>("keep")
  const [basePolicy, setBasePolicy] = useState<PolicyDraft>(() => draftFromPolicy(folder.rescanPolicy, defaultHours))
  const [policy, setPolicyDraft] = useState<PolicyDraft>(basePolicy)
  const [baseWatch, setBaseWatch] = useState<WatchDraft>(() => watchDraftOf(folder))
  const [watch, setWatch] = useState<WatchDraft>(baseWatch)
  const [error, setError] = useState<{ axis: SaveAxis; message: string } | null>(null)
  const [busy, setBusy] = useState(false)

  const fieldsOf = (mode: string) => detectors?.find((d) => d.id === mode)?.optionFields ?? []
  /**
   * 입력 문자열 → 저장할 옵션 + 칸별 오류. 정수 칸은 기본값과 같으면 뺀다, 참조 칸(원격 접속)은 필수이며 항상 저장한다.
   * 공통 옵션은 끌 때만 `false` 로 담는다 (자동 모드에서도 저장된다).
   */
  const readOptions = (draft: WatchDraft) => {
    const errors: Record<string, string> = {}
    const values: WatchOptions = {}
    const fields = detectors === undefined ? undefined : fieldsOf(draft.mode)
    if (fields === undefined) {
      // 감지기 목록을 아직 못 읽었다 — 저장된 값을 그대로 둔다 (검증은 목록이 온 뒤)
      for (const [k, v] of Object.entries(draft.options)) if (Number.isInteger(Number(v)) && v.trim() !== "") values[k] = Number(v)
    } else {
      for (const f of fields) {
        const text = draft.options[f.key] ?? (f.kind === "ref" ? "" : String(f.default))
        const n = Number(text)
        if (f.kind === "bool") continue
        if (f.kind === "ref") {
          if (text.trim() === "" || !Number.isInteger(n) || n < 1) errors[f.key] = t("settings.folders.detection.errorSourceRequired")
          else values[f.key] = n
        } else if (text.trim() === "" || !Number.isInteger(n) || n < f.min || n > f.max) errors[f.key] = t("settings.errorRange", { min: f.min, max: f.max })
        else if (n !== f.default) values[f.key] = n
      }
    }
    if (!draft.exclusive) values[EXCLUSIVE_KEY] = false
    return { errors, values: Object.keys(values).length > 0 ? values : null }
  }
  const current = readOptions(watch)
  const base = readOptions(baseWatch)
  const policyErrors = validateDraft(policy)

  const trimmedAlias = alias.trim()
  const aliasTooLong = [...trimmedAlias].length > FOLDER_ALIAS_MAX
  const aliasChanged = trimmedAlias !== aliasBase
  const typesNone = noMediaTypeSelected(types)
  const typesChanged = MEDIA_TYPE_ORDER.some((k) => typesBase[k] !== types[k])
  const excluded = MEDIA_TYPE_ORDER.filter((k) => typesBase[k] && !types[k])
  const nextPolicy = policyFromDraft(policy, basePolicy)
  const policyChanged = nextPolicy !== "unchanged"
  const modeValue = watch.mode === WATCH_AUTO ? null : watch.mode
  const optionsValue = current.values
  const watchChanged = watch.mode !== baseWatch.mode || JSON.stringify(optionsValue) !== JSON.stringify(base.values)

  const dirty = aliasChanged || typesChanged || policyChanged || watchChanged
  const invalid = aliasTooLong || typesNone || Object.keys(current.errors).length > 0 || Object.keys(policyErrors).length > 0
  const noAutoWarning =
    policy.kind === "manual" && (modeValue === "scan_only" || (modeValue === folder.watchMode && folder.watchActive === "scan_only"))

  /** 바뀐 축을 순서대로 저장한다. 모두 성공하면 true (호출부가 닫는다). */
  const save = async (): Promise<boolean> => {
    if (!dirty || invalid || busy) return false
    setBusy(true)
    setError(null)
    let axis: SaveAxis = "alias"
    let typesSaved = false
    try {
      if (aliasChanged) {
        await setAlias.mutateAsync({ folderId: folder.folderId, alias: trimmedAlias === "" ? null : trimmedAlias })
        setAliasBase(trimmedAlias)
      }
      axis = "types"
      if (typesChanged) {
        await setTypes.mutateAsync({ folderId: folder.folderId, mediaTypes: types, mode: excluded.length > 0 ? typeMode : "keep" })
        setTypesBase(types)
        typesSaved = true
      }
      axis = "policy"
      if (nextPolicy !== "unchanged") {
        await setPolicy.mutateAsync({ folderId: folder.folderId, policy: nextPolicy })
        setBasePolicy(policy)
      }
      axis = "watch"
      if (watchChanged) {
        await setMode.mutateAsync({ folderId: folder.folderId, mode: modeValue, options: optionsValue })
        setBaseWatch(watch)
      }
      // 스캔 대상 변경은 자체 toast(유지/정리 결과)가 이미 떴으므로 한 번 더 띄우지 않는다
      if (!typesSaved) toast.success(t("settings.folders.dialog.saved"))
      return true
    } catch (err) {
      setError({ axis, message: t(errorKey(err)) })
      return false
    } finally {
      setBusy(false)
    }
  }

  const clearError = (a: SaveAxis) => setError((e) => (e?.axis === a ? null : e))

  return {
    dirty,
    invalid,
    busy,
    error,
    save,
    alias: {
      value: alias,
      onChange: (v: string) => {
        setAliasDraft(v)
        clearError("alias")
      },
      tooLong: aliasTooLong,
    },
    types: {
      value: types,
      base: typesBase,
      onChange: (v: MediaTypeSet) => {
        setTypesDraft(v)
        clearError("types")
      },
      mode: typeMode,
      onModeChange: setTypeMode,
      excluded,
      none: typesNone,
    },
    watch: {
      draft: watch,
      onModeChange: (mode: string) => {
        // 감지기 전용 옵션만 비운다 — 공통 옵션(복사 완료 확인)은 감지기와 무관하므로 유지한다
        setWatch((w) => ({ mode, options: {}, exclusive: w.exclusive }))
        clearError("watch")
      },
      onExclusiveChange: (exclusive: boolean) => {
        setWatch((w) => ({ ...w, exclusive }))
        clearError("watch")
      },
      onOptionChange: (key: string, value: string) => setWatch((w) => ({ ...w, options: { ...w.options, [key]: value } })),
      optionErrors: current.errors,
    },
    policy: {
      draft: policy,
      base: basePolicy,
      errors: policyErrors,
      changed: !sameDraft(policy, basePolicy),
      noAutoWarning,
      onChange: (patch: Partial<PolicyDraft>) => {
        setPolicyDraft((d) => ({ ...d, ...patch }))
        clearError("policy")
      },
    },
  }
}
