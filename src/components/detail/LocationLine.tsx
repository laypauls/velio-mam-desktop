import { Folder } from "lucide-react"
import { useTranslation } from "react-i18next"
import { useFolders } from "@/hooks/useLibrary"
import type { AssetDetail } from "@/lib/ipc-contract"
import { CopyTextButton } from "./CopyTextButton"
import { locationText } from "./location"

/** 위치 한 줄(별칭 › 하위 경로) + 전체 경로 복사. 전체 경로는 툴팁으로도 보인다. */
export function LocationLine({ data }: { data: AssetDetail }) {
  const { t } = useTranslation()
  const folders = useFolders().data
  const folder = data.folderId === null ? null : (folders?.find((f) => f.folderId === data.folderId) ?? null)
  const text = locationText(data.path, folder)
  return (
    <div className="group flex items-center gap-1 text-xs text-muted-foreground">
      <Folder aria-hidden="true" className="size-3.5 shrink-0" />
      <span className="sr-only">{t("detail.location")}</span>
      <span className="min-w-0 flex-1 truncate select-text" title={data.path}>
        {text}
      </span>
      <CopyTextButton label={t("detail.copyPath")} text={data.path} doneKey="detail.copiedPath" />
    </div>
  )
}
