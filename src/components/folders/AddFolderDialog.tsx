import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import type { AddFolderFlow } from "@/hooks/useAddFolder"
import { errorKey } from "@/lib/errors"
import { ALL_MEDIA_TYPES, MediaTypeFields, noMediaTypeSelected } from "./MediaTypeFields"

function AddFolderForm({ flow, path }: { flow: AddFolderFlow; path: string }) {
  const { t } = useTranslation()
  const [types, setTypes] = useState(ALL_MEDIA_TYPES)
  const { add } = flow
  const none = noMediaTypeSelected(types)
  const error = none ? t("folders.types.noneSelected") : add.isError ? t(errorKey(add.error)) : null

  const submit = () => {
    if (none || add.isPending) return
    add.mutate({ path, mediaTypes: types }, { onSuccess: flow.close })
  }

  return (
    <>
      <MediaTypeFields
        value={types}
        onChange={(next) => {
          setTypes(next)
          if (add.isError) add.reset()
        }}
        disabled={add.isPending}
        error={error}
      />
      <DialogFooter>
        <Button type="button" variant="outline" disabled={add.isPending} onClick={flow.close}>
          {t("common.cancel")}
        </Button>
        <Button type="button" aria-busy={add.isPending} disabled={none || add.isPending} onClick={submit}>
          {t("folders.add.confirm")}
        </Button>
      </DialogFooter>
    </>
  )
}

/** 폴더 선택 직후 대상 유형을 고르는 공용 Dialog — 4개 진입점(사이드바·빈 갤러리·온보딩·설정)이 같은 흐름을 쓴다. */
export function AddFolderDialog({ flow }: { flow: AddFolderFlow }) {
  const { t } = useTranslation()
  return (
    <Dialog open={flow.path !== null} onOpenChange={(o) => !o && flow.close()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("folders.add.title")}</DialogTitle>
          <DialogDescription className="break-all">{flow.path}</DialogDescription>
        </DialogHeader>
        {/* key 로 선택한 경로마다 체크 상태·오류를 새로 시작한다 */}
        {flow.path !== null && <AddFolderForm key={flow.path} flow={flow} path={flow.path} />}
      </DialogContent>
    </Dialog>
  )
}
