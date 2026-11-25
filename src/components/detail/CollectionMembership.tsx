import { Pencil, Plus } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { useCollections } from "@/hooks/useLibrary"
import { useAddToCollection, useRemoveFromCollection, useToastError } from "@/hooks/useUserData"
import type { CollectionRef } from "@/lib/ipc-contract"
import { useActionsStore } from "@/stores"

/** 상세 패널 컬렉션 소속 — 칩 목록 + 체크박스 Popover(소속 편집, 새 컬렉션). */
export function CollectionMembership({ assetId, memberOf }: { assetId: number; memberOf: CollectionRef[] }) {
  const { t } = useTranslation()
  const collections = useCollections()
  const add = useAddToCollection()
  const remove = useRemoveFromCollection()
  const toastError = useToastError()
  const openForm = useActionsStore((s) => s.openCollectionForm)
  const member = new Set(memberOf.map((c) => c.collectionId))

  const toggle = (collectionId: number, checked: boolean) => {
    const vars = { collectionId, assetIds: [assetId] }
    if (checked) add.mutate(vars, { onError: toastError })
    else remove.mutate(vars, { onError: toastError })
  }

  return (
    <section className="flex flex-col gap-1.5">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-xs font-medium text-muted-foreground">{t("detail.collections")}</h3>
        <Popover>
          <PopoverTrigger asChild>
            <Button size="xs" variant="ghost" aria-label={t("collections.edit")}>
              <Pencil aria-hidden="true" />
              {t("detail.collectionsEdit")}
            </Button>
          </PopoverTrigger>
        <PopoverContent align="start" className="flex w-64 flex-col gap-2" aria-label={t("collections.edit")}>
          {collections.data && collections.data.length > 0 ? (
            <ul className="flex max-h-60 flex-col gap-1 overflow-y-auto">
              {collections.data.map((c) => (
                <li key={c.collectionId} className="flex items-center gap-2">
                  <Checkbox
                    id={`coll-${c.collectionId}`}
                    checked={member.has(c.collectionId)}
                    disabled={add.isPending || remove.isPending}
                    onCheckedChange={(v) => toggle(c.collectionId, v === true)}
                  />
                  <Label htmlFor={`coll-${c.collectionId}`} className="min-w-0 flex-1 truncate">
                    {c.name}
                  </Label>
                </li>
              ))}
            </ul>
          ) : (
            <p className="text-sm break-keep text-muted-foreground">{t("sidebar.collectionsEmpty")}</p>
          )}
          <Button size="sm" variant="ghost" className="justify-start" onClick={() => openForm({ mode: "create", assetIds: [assetId] })}>
            <Plus aria-hidden="true" />
            {t("collections.new")}
          </Button>
        </PopoverContent>
        </Popover>
      </div>
      {memberOf.length === 0 ? (
        <p className="text-sm break-keep text-muted-foreground">{t("detail.collectionsEmpty")}</p>
      ) : (
        <ul aria-label={t("detail.collections")} className="flex flex-wrap gap-1">
          {memberOf.map((c) => (
            <li key={c.collectionId}>
              <Badge variant="secondary" className="rounded-sm">
                {c.name}
              </Badge>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
