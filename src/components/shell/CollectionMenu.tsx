import { MoreHorizontal, Pencil, Play, Trash2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { useStartPlayback } from "@/hooks/usePlayQueue"
import type { CollectionRef } from "@/lib/ipc-contract"
import { useActionsStore } from "@/stores"

/** 사이드바 컬렉션 항목의 ⋯ 메뉴 — 재생, 이름 변경, 삭제(확인 AlertDialog). */
export function CollectionMenu({ collection }: { collection: CollectionRef }) {
  const { t } = useTranslation()
  const openForm = useActionsStore((s) => s.openCollectionForm)
  const askDelete = useActionsStore((s) => s.askDeleteCollection)
  const start = useStartPlayback()
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="icon-xs" variant="ghost" aria-label={t("collections.menu", { name: collection.name })}>
          <MoreHorizontal aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        <DropdownMenuItem onSelect={() => void start(collection.collectionId)}>
          <Play aria-hidden="true" />
          {t("collections.play")}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => openForm({ mode: "rename", collection })}>
          <Pencil aria-hidden="true" />
          {t("collections.rename")}
        </DropdownMenuItem>
        <DropdownMenuItem variant="destructive" onSelect={() => askDelete(collection)}>
          <Trash2 aria-hidden="true" className="text-destructive" />
          {t("collections.delete")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
