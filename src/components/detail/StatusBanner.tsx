import { History, TriangleAlert, Unplug, type LucideIcon } from "lucide-react"
import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useDisplayName } from "@/hooks/useDisplayName"
import { useRescanFolder } from "@/hooks/useFolderAdmin"
import type { AssetDetail } from "@/lib/ipc-contract"
import { useSelectionStore } from "@/stores"

function Banner({ role, icon: Icon, title, body, children }: { role: "status" | "note"; icon: LucideIcon; title: string; body: string; children?: ReactNode }) {
  return (
    <div role={role} className="flex flex-col gap-2 rounded-lg border border-border bg-muted/40 p-3 text-sm">
      <p className="flex items-start gap-2 font-medium break-keep">
        <Icon aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
        {title}
      </p>
      <p className="text-xs break-keep text-muted-foreground">{body}</p>
      {children}
    </div>
  )
}

/**
 * 정상(online·active)이면 아무것도 그리지 않는다. 문제 상태(오프라인·파일 없음·이전 버전)만 원인과 대처를 배너로 보여 준다.
 * "이전 버전" 안내(PL6)는 새 버전으로 이동하는 버튼을 포함한다.
 */
export function StatusBanner({ detail }: { detail: AssetDetail }) {
  const { t } = useTranslation()
  const selectOnly = useSelectionStore((s) => s.selectOnly)
  const { nameOf } = useDisplayName()
  const rescan = useRescanFolder()
  const newer = detail.newerVersion
  return (
    <>
      {detail.availability === "offline" && (
        <Banner
          role="status"
          icon={Unplug}
          title={t("detail.banner.offlineTitle")}
          body={detail.volumeLabel ? t("detail.banner.offlineBodyNamed", { volume: detail.volumeLabel }) : t("detail.banner.offlineBody")}
        />
      )}
      {detail.availability === "missing" && (
        <Banner role="status" icon={TriangleAlert} title={t("detail.banner.missingTitle")} body={t("detail.banner.missingBody")}>
          {detail.folderId !== null && (
            <Button size="xs" variant="outline" className="self-start" disabled={rescan.isPending} onClick={() => rescan.mutate(detail.folderId as number)}>
              {t("detail.banner.rescan")}
            </Button>
          )}
        </Banner>
      )}
      {detail.status === "superseded" && (
        <Banner role="note" icon={History} title={t("detail.supersededNotice.title")} body={t("detail.supersededNotice.body")}>
          {newer && (
            <Button size="xs" variant="outline" className="self-start" onClick={() => selectOnly(newer.assetId)}>
              {t("detail.supersededNotice.open", { name: nameOf(newer) })}
            </Button>
          )}
        </Banner>
      )}
    </>
  )
}
