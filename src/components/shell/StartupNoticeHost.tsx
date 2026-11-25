import { useQuery } from "@tanstack/react-query"
import { TriangleAlert } from "lucide-react"
import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { qk } from "@/hooks/keys"
import { useApi } from "@/lib/api"
import { formatDateTime } from "@/lib/format"
import type { StartupNotice } from "@/lib/ipc-contract"

type Recovered = Extract<StartupNotice, { kind: "recovered" }>

function RecoveredDialog({ notice, onClose }: { notice: Recovered; onClose: () => void }) {
  const { t } = useTranslation()
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <TriangleAlert aria-hidden="true" className="size-5 text-destructive" />
            {t("startup.recovered.title")}
          </DialogTitle>
          <DialogDescription className="break-keep">
            {notice.backupFile !== null && notice.createdAt !== null
              ? t("startup.recovered.withBackup", { date: formatDateTime(notice.createdAt) })
              : t("startup.recovered.noBackup")}
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-1 text-sm">
          <p className="break-keep">{t("startup.recovered.corruptFile")}</p>
          <p className="rounded-sm bg-muted px-2 py-1 font-mono text-xs break-all">{notice.corruptFile}</p>
        </div>
        {notice.consentReset && <p className="text-sm break-keep">{t("startup.recovered.consentReset")}</p>}
        <DialogFooter>
          <Button onClick={onClose}>{t("common.confirm")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/** 기동 후 첫 화면에서 `take_startup_notice` 를 1회 읽어 복원(toast)·자동 복구(Dialog) 결과를 알린다. */
export function StartupNoticeHost() {
  const { t } = useTranslation()
  const api = useApi()
  // 읽으면 지워지는 command — 재시도·재조회 없이 한 번만 (StrictMode 이중 effect 에도 query 가 합쳐 준다)
  const { data } = useQuery({
    queryKey: qk.startupNotice,
    queryFn: () => api.takeStartupNotice(),
    staleTime: Infinity,
    gcTime: Infinity,
    retry: false,
    refetchOnMount: false,
    refetchOnReconnect: false,
    refetchOnWindowFocus: false,
  })
  const [recovered, setRecovered] = useState<Recovered | null>(null)
  const handled = useRef(false)

  useEffect(() => {
    if (!data || handled.current) return
    handled.current = true
    if (data.kind === "restored") toast.success(t("startup.restored", { date: formatDateTime(data.createdAt) }))
    else setRecovered(data)
  }, [data, t])

  return recovered && <RecoveredDialog notice={recovered} onClose={() => setRecovered(null)} />
}
