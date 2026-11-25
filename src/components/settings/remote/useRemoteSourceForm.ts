import { useState } from "react"
import { useSaveRemoteSource, useTestRemoteSource } from "@/hooks/useRemoteSources"
import { errorKey, isCommandError } from "@/lib/errors"
import type { PathMapping, RemoteSource, RemoteSourceKind, RemoteTestResult } from "@/lib/ipc-contract"
import { draftOf, emptyDraft, fieldOfError, toInput, validateDraft, type RemoteDraft, type RemoteErrors } from "./remote-form"

/** 접속 폼의 입력·점검·접속 시험·지문 확인·저장. 비밀은 `draft.secret` 에만 있고 저장·시험 호출 뒤에도 폼 안에서만 산다. */
export function useRemoteSourceForm(stored: RemoteSource | null, kind: RemoteSourceKind, onSaved: (saved: RemoteSource) => void) {
  const save = useSaveRemoteSource()
  const test = useTestRemoteSource()
  const [draft, setDraft] = useState<RemoteDraft>(() => (stored ? draftOf(stored) : emptyDraft(kind)))
  const [errors, setErrors] = useState<RemoteErrors>({})
  /** 필드에 묶이지 않는 저장·시험 실패 (번역 키) */
  const [failure, setFailure] = useState<string | null>(null)
  const [result, setResult] = useState<RemoteTestResult | null>(null)
  const busy = save.isPending || test.isPending

  const clear = (field: keyof RemoteErrors) => setErrors((e) => (e[field] === undefined ? e : { ...e, [field]: undefined }))
  const reportFailure = (err: unknown) => {
    const field = fieldOfError(err)
    if (field !== null) setErrors((e) => ({ ...e, [field]: field }))
    else if (isCommandError(err) && err.code === "already_registered") setErrors((e) => ({ ...e, name: "nameTaken" }))
    else setFailure(errorKey(err))
  }

  const set = (patch: Partial<RemoteDraft>) => {
    // 호스트·포트가 바뀌면 확인한 지문은 다른 장비의 것이다 — 다시 확인해야 한다
    const moved = (patch.host !== undefined && patch.host !== draft.host) || (patch.port !== undefined && patch.port !== draft.port)
    setDraft((d) => ({ ...d, ...patch, ...(moved ? { hostKey: null } : {}) }))
    if (moved) setResult(null)
    setFailure(null)
    for (const key of Object.keys(patch)) clear(key as keyof RemoteErrors)
  }
  const setPathMap = (rows: PathMapping[]) => set({ pathMap: rows })

  /** 접속 시험 — 지문을 확인하기 전에는 인증하지 않고 지문만 돌아온다 */
  const runTest = async () => {
    const found = validateDraft(draft, stored, draft.hostKey !== null)
    setErrors(found)
    setFailure(null)
    if (Object.keys(found).length > 0) return
    try {
      // 지문을 확인하기 전에는 인증하지 않는다 — 비밀도 보내지 않는다
      const input = draft.hostKey === null ? { ...toInput(draft), secret: null } : toInput(draft)
      setResult(await test.mutateAsync({ sourceId: stored?.sourceId ?? null, input }))
    } catch (err) {
      setResult(null)
      reportFailure(err)
    }
  }

  /** 사용자가 지문을 눈으로 확인했다 — 이 지문을 저장 대상으로 고정한다 (다시 시험해야 인증한다) */
  const confirmHostKey = () => {
    if (result?.fingerprint == null) return
    setDraft((d) => ({ ...d, hostKey: result.fingerprint }))
    setResult(null)
  }

  const submit = async () => {
    const found = validateDraft(draft, stored)
    setErrors(found)
    setFailure(null)
    if (Object.keys(found).length > 0) return
    try {
      onSaved(await save.mutateAsync({ sourceId: stored?.sourceId ?? null, input: toInput(draft) }))
    } catch (err) {
      reportFailure(err)
    }
  }

  return { draft, errors, failure, result, busy, testing: test.isPending, saving: save.isPending, set, setPathMap, runTest, confirmHostKey, submit }
}
