import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { ScrollArea } from "@/components/ui/scroll-area"
import { useRetryScanErrors, useScanErrors } from "@/hooks/useFolderAdmin"
import { useFolders } from "@/hooks/useLibrary"
import { errorKey } from "@/lib/errors"
import { formatDateTime, formatNumber } from "@/lib/format"

/** 스캔 오류 목록(페이지 단위) + 선택/전체 재시도. */
export function ScanErrorsSection() {
  const { t } = useTranslation()
  const query = useScanErrors(true)
  const retry = useRetryScanErrors()
  const [selected, setSelected] = useState<Set<number>>(new Set())
  const items = query.data?.pages.flatMap((p) => p.items) ?? []
  const total = query.data?.pages[0]?.total ?? 0
  const folders = useFolders().data
  const aliasById = new Map(folders?.flatMap((f) => (f.alias ? [[f.folderId, f.alias] as const] : [])))

  const toggle = (id: number) =>
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })

  return (
    <section aria-label={t("settings.folders.errors")} className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <h3 className="text-sm font-semibold">{t("settings.folders.errorsTitle", { count: formatNumber(total) })}</h3>
        <div className="ml-auto flex gap-2">
          <Button
            size="xs"
            variant="outline"
            disabled={selected.size === 0 || retry.isPending}
            onClick={() => retry.mutate([...selected], { onSuccess: () => setSelected(new Set()) })}
          >
            {t("settings.folders.retrySelected")}
          </Button>
          <Button size="xs" variant="outline" disabled={total === 0 || retry.isPending} onClick={() => retry.mutate(null)}>
            {t("settings.folders.retryAll")}
          </Button>
        </div>
      </div>
      {query.isError && (
        <p role="alert" className="text-xs text-destructive">
          {t(errorKey(query.error))}
        </p>
      )}
      {!query.isPending && items.length === 0 && !query.isError && (
        <p className="text-sm text-muted-foreground">{t("settings.folders.errorsEmpty")}</p>
      )}
      {items.length > 0 && (
        <ScrollArea className="max-h-40 rounded-lg border border-border">
          <ul className="flex flex-col divide-y divide-border">
            {items.map((e) => (
              <li key={e.errorId} className="flex items-start gap-2 px-3 py-2">
                <Checkbox
                  aria-label={t("settings.folders.selectError", { path: e.path })}
                  checked={selected.has(e.errorId)}
                  onCheckedChange={() => toggle(e.errorId)}
                  className="mt-0.5"
                />
                <div className="flex min-w-0 flex-col gap-0.5">
                  <span className="text-sm break-all">{e.path}</span>
                  <span className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground break-keep">
                    {aliasById.get(e.folderId) && (
                      <Badge variant="outline" className="rounded-sm">
                        {aliasById.get(e.folderId)}
                      </Badge>
                    )}
                    {t("settings.folders.errorMeta", { code: e.errorCode, attempts: e.attempts, at: formatDateTime(e.lastAt) })}
                  </span>
                </div>
              </li>
            ))}
          </ul>
        </ScrollArea>
      )}
      {query.hasNextPage && (
        <Button size="xs" variant="ghost" className="self-start" disabled={query.isFetchingNextPage} onClick={() => void query.fetchNextPage()}>
          {t("settings.folders.loadMore")}
        </Button>
      )}
    </section>
  )
}
