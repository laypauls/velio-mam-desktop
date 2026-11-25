import { X } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Badge } from "@/components/ui/badge"
import { useToastError, useUpdateTags } from "@/hooks/useUserData"
import type { TagRef } from "@/lib/ipc-contract"
import { TagInput } from "./TagInput"

/** 상세 패널 태그 편집 — 칩(제거 버튼) + 추가 입력(자동완성). */
export function TagEditor({ assetId, tags }: { assetId: number; tags: TagRef[] }) {
  const { t } = useTranslation()
  const update = useUpdateTags()
  const toastError = useToastError()

  return (
    <div className="flex flex-col gap-2">
      {tags.length > 0 && (
        <ul aria-label={t("detail.tags")} className="flex flex-wrap gap-1">
          {tags.map((tag) => (
            <li key={tag.tagId}>
              <Badge variant="secondary" className="gap-1 rounded-sm pr-0.5">
                {tag.name}
                <button
                  type="button"
                  aria-label={t("tags.remove", { name: tag.name })}
                  disabled={update.isPending}
                  onClick={() =>
                    update.mutate({ assetIds: [assetId], add: [], remove: [tag.tagId] }, { onError: toastError })
                  }
                  className="rounded-sm p-0.5 hover:bg-foreground/10 focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
                >
                  <X aria-hidden="true" className="size-3" />
                </button>
              </Badge>
            </li>
          ))}
        </ul>
      )}
      <TagInput
        label={t("tags.add")}
        existing={tags.map((x) => x.name)}
        onAdd={(name) => update.mutateAsync({ assetIds: [assetId], add: [name], remove: [] })}
      />
    </div>
  )
}
