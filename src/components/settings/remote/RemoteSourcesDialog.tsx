import { Plus } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { useRemoteSources } from "@/hooks/useRemoteSources"
import { errorKey } from "@/lib/errors"
import type { RemoteSource, RemoteSourceKind } from "@/lib/ipc-contract"
import { DeleteRemoteSourceDialog } from "./DeleteRemoteSourceDialog"
import { RemoteSourceForm } from "./RemoteSourceForm"
import { RemoteSourceList } from "./RemoteSourceList"

/** 목록 보기 / 새 접속(`"new"`) / 수정(접속) */
type View = "list" | "new" | RemoteSource

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** 새 접속의 처음 종류 — 감지 방식에서 열었으면 그 방식이 쓰는 종류 */
  initialKind: RemoteSourceKind
  /** 새 접속을 저장했다 (감지 방식 폼이 방금 만든 접속을 고르게 한다) */
  onCreated?: (source: RemoteSource) => void
}

/** 원격 스토리지 접속 관리 — 목록과 만들기·수정 폼, 삭제 확인. 접속 하나를 여러 폴더가 공유한다. */
export function RemoteSourcesDialog({ open, onOpenChange, initialKind, onCreated }: Props) {
  const { t } = useTranslation()
  const sources = useRemoteSources()
  const [view, setView] = useState<View>("list")
  const [deleting, setDeleting] = useState<RemoteSource | null>(null)

  const close = (o: boolean) => {
    if (!o) setView("list")
    onOpenChange(o)
  }
  const saved = (s: RemoteSource) => {
    toast.success(t("settings.folders.remote.saved", { name: s.name }))
    if (view === "new") onCreated?.(s)
    setView("list")
  }
  const editing = view !== "list" && view !== "new"

  return (
    <>
      <Dialog open={open} onOpenChange={close}>
        <DialogContent className="max-h-[calc(100dvh-2rem)] grid-rows-[auto_minmax(0,1fr)] sm:max-w-xl">
          <DialogHeader>
            <DialogTitle>{t(view === "list" ? "settings.folders.remote.title" : editing ? "settings.folders.remote.titleEdit" : "settings.folders.remote.titleNew")}</DialogTitle>
            <DialogDescription className="break-keep">{t("settings.folders.remote.description")}</DialogDescription>
          </DialogHeader>
          {view === "list" ? (
            <div className="flex min-h-0 flex-col gap-4">
              {/* 본문만 스크롤 — relative 로 내부 absolute/sr-only 자손이 바깥 조상 기준으로 배치되는 것을 막는다 */}
              <div className="relative flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto">
                {sources.isError ? (
                  <p role="alert" className="text-sm text-destructive">
                    {t(errorKey(sources.error))}
                  </p>
                ) : (
                  <RemoteSourceList sources={sources.data ?? []} onEdit={setView} onDelete={setDeleting} />
                )}
              </div>
              <DialogFooter>
                <Button type="button" variant="outline" onClick={() => setView("new")}>
                  <Plus aria-hidden="true" />
                  {t("settings.folders.remote.add")}
                </Button>
                <Button type="button" onClick={() => close(false)}>
                  {t("common.close")}
                </Button>
              </DialogFooter>
            </div>
          ) : (
            // key 로 대상마다 입력 상태(비밀 포함)를 새로 시작한다
            <RemoteSourceForm
              key={editing ? view.sourceId : "new"}
              stored={editing ? view : null}
              initialKind={initialKind}
              onCancel={() => setView("list")}
              onSaved={saved}
            />
          )}
        </DialogContent>
      </Dialog>
      <DeleteRemoteSourceDialog source={deleting} onClose={() => setDeleting(null)} />
    </>
  )
}
