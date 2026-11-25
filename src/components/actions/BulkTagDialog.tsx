import { X } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { TagInput } from "@/components/detail/TagInput"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import { ScrollArea } from "@/components/ui/scroll-area"
import { useTags } from "@/hooks/useLibrary"
import { useUpdateTags } from "@/hooks/useUserData"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import { useActionsStore } from "@/stores"

function BulkTagBody({ assetIds }: { assetIds: number[] }) {
  const { t } = useTranslation()
  const close = useActionsStore((s) => s.closeBulkTags)
  const tags = useTags()
  const update = useUpdateTags()
  const [add, setAdd] = useState<string[]>([])
  const [remove, setRemove] = useState<Set<number>>(new Set())
  const [error, setError] = useState<string | null>(null)

  const stage = (name: string) => {
    setAdd((prev) => (prev.some((n) => n.toLowerCase() === name.toLowerCase()) ? prev : [...prev, name]))
  }
  const toggleRemove = (tagId: number, on: boolean) =>
    setRemove((prev) => {
      const next = new Set(prev)
      if (on) next.add(tagId)
      else next.delete(tagId)
      return next
    })

  const apply = async () => {
    setError(null)
    try {
      await update.mutateAsync({ assetIds, add, remove: [...remove] })
      toast.success(t("tags.bulkApplied", { count: formatNumber(assetIds.length) }))
      close()
    } catch (e) {
      setError(t(errorKey(e)))
    }
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>{t("tags.bulkTitle")}</DialogTitle>
        <DialogDescription>{t("tags.bulkDescription", { count: formatNumber(assetIds.length) })}</DialogDescription>
      </DialogHeader>
      <div className="flex flex-col gap-4">
        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-medium">{t("tags.bulkAddSection")}</h3>
          <TagInput label={t("tags.add")} existing={add} onAdd={stage} />
          {add.length > 0 && (
            <ul aria-label={t("tags.bulkStaged")} className="flex flex-wrap gap-1">
              {add.map((n) => (
                <li key={n}>
                  <Badge variant="secondary" className="gap-1 rounded-sm pr-0.5">
                    {n}
                    <button
                      type="button"
                      aria-label={t("tags.unstage", { name: n })}
                      onClick={() => setAdd((prev) => prev.filter((x) => x !== n))}
                      className="rounded-sm p-0.5 hover:bg-foreground/10 focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
                    >
                      <X aria-hidden="true" className="size-3" />
                    </button>
                  </Badge>
                </li>
              ))}
            </ul>
          )}
        </section>
        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-medium">{t("tags.bulkRemoveSection")}</h3>
          {tags.data && tags.data.length > 0 ? (
            <ScrollArea className="h-40 rounded-lg border border-border">
              <ul aria-label={t("tags.bulkRemoveSection")} className="flex flex-col gap-1 p-2">
                {tags.data.map((tag) => (
                  <li key={tag.tagId} className="flex items-center gap-2">
                    <Checkbox
                      id={`bulk-rm-${tag.tagId}`}
                      checked={remove.has(tag.tagId)}
                      onCheckedChange={(v) => toggleRemove(tag.tagId, v === true)}
                    />
                    <Label htmlFor={`bulk-rm-${tag.tagId}`} className="min-w-0 flex-1 truncate">
                      {tag.name}
                    </Label>
                  </li>
                ))}
              </ul>
            </ScrollArea>
          ) : (
            <p className="text-sm break-keep text-muted-foreground">{t("tags.empty.description")}</p>
          )}
        </section>
        {error !== null && (
          <p role="alert" className="text-xs break-keep text-destructive">
            {error}
          </p>
        )}
      </div>
      <DialogFooter>
        <Button variant="outline" onClick={close}>
          {t("common.cancel")}
        </Button>
        <Button disabled={update.isPending || (add.length === 0 && remove.size === 0)} onClick={() => void apply()}>
          {t("tags.bulkApply")}
        </Button>
      </DialogFooter>
    </>
  )
}

/** 일괄 태그 — 추가할 태그를 담고, 제거할 태그를 체크해 한 번에 적용한다. */
export function BulkTagDialog() {
  const ids = useActionsStore((s) => s.bulkTagIds)
  const close = useActionsStore((s) => s.closeBulkTags)
  return (
    <Dialog open={ids !== null} onOpenChange={(o) => !o && close()}>
      <DialogContent className="sm:max-w-lg">{ids !== null && <BulkTagBody assetIds={ids} />}</DialogContent>
    </Dialog>
  )
}
