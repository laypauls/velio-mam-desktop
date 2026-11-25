import { Pencil, X } from "lucide-react"
import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useSettings } from "@/hooks/useSettings"
import { useSetAssetTitle } from "@/hooks/useUserData"
import { effectiveTitle, splitFileName } from "@/lib/displayName"
import { errorKey } from "@/lib/errors"

export const TITLE_MAX_LENGTH = 200

interface Props {
  assetId: number
  title: string | null
  fileName: string
  /** 실제 파일명 옆에 놓을 버튼(파일명 복사) — 제목이 없거나 편집이 꺼져 있으면 제목(=파일명) 옆에 놓인다 */
  fileNameAction?: ReactNode
}

/**
 * 상세 패널 이름 영역. 제목 편집이 꺼져 있으면 파일명만 보여 준다.
 * 켜져 있으면 기본은 보기 상태(제목 + 파일명 줄)이고, ✎ 버튼 또는 제목 더블클릭으로 편집 상태(입력창)에 들어간다.
 * 편집: Enter/blur 저장, Esc 되돌리기, ✕ = 제목 해제.
 */
export function TitleEditor({ assetId, title, fileName, fileNameAction }: Props) {
  const { t } = useTranslation()
  const enabled = useSettings().data?.titleEditEnabled ?? false
  const setTitle = useSetAssetTitle()
  const errorId = useId()
  // 저장된 제목이 없으면 확장자 없는 파일명을 채워 보여 준다 (표시용 — 저장하지 않는다)
  const effective = effectiveTitle({ title, fileName })
  const [value, setValue] = useState(effective)
  const [error, setError] = useState<string | null>(null)
  const [editing, setEditing] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)
  const pencilRef = useRef<HTMLButtonElement>(null)
  // 편집을 떠나는 중이면 입력창 제거로 발생하는 blur 가 다시 저장하지 않게 한다
  const leaving = useRef(false)
  const refocusPencil = useRef(false)

  useEffect(() => {
    setValue(effective)
    setError(null)
  }, [assetId, effective])

  useEffect(() => {
    setEditing(false)
  }, [assetId])

  useEffect(() => {
    if (editing) {
      leaving.current = false
      inputRef.current?.focus()
      inputRef.current?.select()
    } else if (refocusPencil.current) {
      refocusPencil.current = false
      pencilRef.current?.focus()
    }
  }, [editing])

  if (!enabled) {
    return (
      <div className="group flex items-start gap-1">
        <h2 className="min-w-0 text-base font-semibold break-all select-text">{fileName}</h2>
        {fileNameAction}
      </div>
    )
  }

  const leave = (refocus: boolean) => {
    leaving.current = true
    refocusPencil.current = refocus
    setEditing(false)
  }

  const commit = (next: string, refocus: boolean) => {
    if (leaving.current) return
    const trimmed = next.trim()
    // 기본값(파일명)이나 저장된 제목과 같으면 저장하지 않고 편집만 끝낸다. 비우면 해제 → 다시 파일명이 채워진다
    if (trimmed === effective) return leave(refocus)
    if (setTitle.isPending) return
    if (trimmed === "") {
      setValue(splitFileName(fileName)[0])
      if (title === null) return leave(refocus)
    }
    setError(null)
    setTitle.mutate(
      { assetId, title: trimmed === "" ? null : trimmed },
      { onSuccess: () => leave(refocus), onError: (e) => setError(t(errorKey(e))) }
    )
  }

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault()
      commit(value, true)
    } else if (e.key === "Escape") {
      e.preventDefault()
      setValue(effective)
      setError(null)
      leave(true)
    }
  }

  const pencil = (
    <Button
      ref={pencilRef}
      type="button"
      variant="ghost"
      size="icon-xs"
      aria-label={t("titleEdit.edit")}
      title={t("titleEdit.edit")}
      onClick={() => setEditing(true)}
      className="shrink-0 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100"
    >
      <Pencil aria-hidden="true" />
    </Button>
  )

  if (!editing) {
    // 제목 유무와 무관하게 같은 트리 위치를 유지한다 — ✎ 가 다시 마운트되면 포커스를 잃는다
    return (
      <div className="flex flex-col gap-1.5">
        <div className="group flex items-start gap-1">
          <h2 className="min-w-0 text-base font-semibold break-all select-text" onDoubleClick={() => setEditing(true)}>
            {title ?? fileName}
          </h2>
          {title === null && fileNameAction}
          {pencil}
        </div>
        {title !== null && <FileNameLine fileName={fileName} action={fileNameAction} label={t("titleEdit.fileName")} />}
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-1">
        <Input
          ref={inputRef}
          value={value}
          maxLength={TITLE_MAX_LENGTH}
          aria-label={t("titleEdit.label")}
          aria-invalid={error !== null}
          aria-describedby={error !== null ? errorId : undefined}
          placeholder={t("titleEdit.placeholder")}
          onChange={(e) => setValue(e.target.value)}
          onBlur={() => commit(value, false)}
          onKeyDown={onKeyDown}
        />
        {title !== null && (
          <Button type="button" variant="ghost" size="icon" aria-label={t("titleEdit.clear")} title={t("titleEdit.clear")} onMouseDown={(e) => e.preventDefault()} onClick={() => commit("", true)}>
            <X aria-hidden="true" />
          </Button>
        )}
      </div>
      {error !== null && (
        <p id={errorId} role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
      <p className="text-xs text-muted-foreground">{t("titleEdit.editHint")}</p>
      <FileNameLine fileName={fileName} action={fileNameAction} label={t("titleEdit.fileName")} />
    </div>
  )
}

function FileNameLine({ fileName, action, label }: { fileName: string; action?: ReactNode; label: string }) {
  return (
    <div className="group flex items-center gap-1 text-xs text-muted-foreground">
      <p className="min-w-0 break-all">
        <span className="font-medium">{label}</span> <span className="select-text">{fileName}</span>
      </p>
      {action}
    </div>
  )
}
