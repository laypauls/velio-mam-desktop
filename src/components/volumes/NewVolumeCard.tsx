import { HardDrive } from "lucide-react"
import { useId, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { ALL_MEDIA_TYPES, MediaTypeFields, noMediaTypeSelected } from "@/components/folders/MediaTypeFields"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { useAddFolderRequest } from "@/hooks/useFolderAdmin"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatBytes } from "@/lib/format"
import type { PendingVolume } from "@/lib/ipc-contract"
import { isInsideMount, volumeTitle } from "@/lib/volumes"

type Scope = "all" | "folder"

interface Props {
  volume: PendingVolume
  /** 다른 곳에서 진행 중인 "나중에/닫기" 동안 버튼을 잠근다 */
  busy: boolean
  onLater: () => void
}

/** 새 디스크 한 장 — 등록 범위(디스크 전체 / 폴더 선택) + 스캔 대상 유형 + [나중에] [추가]. 등록되면 백엔드 이벤트로 카드가 사라진다. */
export function NewVolumeCard({ volume, busy, onLater }: Props) {
  const { t } = useTranslation()
  const api = useApi()
  const add = useAddFolderRequest()
  const [scope, setScope] = useState<Scope>("all")
  const [folder, setFolder] = useState<string | null>(null)
  const [types, setTypes] = useState(ALL_MEDIA_TYPES)
  const uid = useId()
  const errId = `${uid}-err`

  const outside = scope === "folder" && folder !== null && !isInsideMount(folder, volume.mountPoint)
  const noTypes = noMediaTypeSelected(types)
  const target = scope === "all" ? volume.mountPoint : folder
  const locked = busy || add.isPending
  const canAdd = !locked && !noTypes && !outside && target !== null

  const pick = async () => {
    try {
      const dir = await api.pickDirectory(volume.mountPoint)
      if (dir !== null) {
        setFolder(dir)
        if (add.isError) add.reset()
      }
    } catch (e) {
      toast.error(t(errorKey(e)))
    }
  }

  const meta = [t(`settings.folders.volume.${volume.kind}`), volume.totalBytes === null ? null : formatBytes(volume.totalBytes)]
    .filter((x) => x !== null)
    .join(" · ")
  return (
    <section aria-label={volumeTitle(volume)} className="flex flex-col gap-3 rounded-lg border border-border p-3">
      <div className="flex items-start gap-2">
        <HardDrive aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        <h4 className="min-w-0 flex-1 text-sm font-medium break-all">{volumeTitle(volume)}</h4>
        <span className="shrink-0 text-xs text-muted-foreground">{meta}</span>
      </div>
      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1 text-sm font-medium">{t("volumes.new.scope")}</legend>
        <RadioGroup
          value={scope}
          onValueChange={(v) => {
            setScope(v as Scope)
            if (add.isError) add.reset()
          }}
          disabled={locked}
          className="gap-2"
        >
          <div className="flex items-center gap-2">
            <RadioGroupItem value="all" id={`${uid}-all`} />
            <Label htmlFor={`${uid}-all`}>{t("volumes.new.scopeAll")}</Label>
          </div>
          <div className="flex items-center gap-2">
            <RadioGroupItem value="folder" id={`${uid}-folder`} />
            <Label htmlFor={`${uid}-folder`}>{t("volumes.new.scopeFolder")}</Label>
          </div>
        </RadioGroup>
        {scope === "folder" && (
          <div className="flex items-center gap-2 pl-6">
            <Input
              readOnly
              value={folder ?? ""}
              placeholder={t("volumes.new.folderPlaceholder")}
              aria-label={t("volumes.new.folderLabel")}
              aria-invalid={outside ? true : undefined}
              aria-describedby={outside ? errId : undefined}
              className="h-8 min-w-0 flex-1 text-xs"
            />
            <Button type="button" size="sm" variant="outline" disabled={locked} onClick={() => void pick()}>
              {t("volumes.new.browse")}
            </Button>
          </div>
        )}
        {outside && (
          <p id={errId} role="alert" className="pl-6 text-xs text-destructive break-keep">
            {t("volumes.new.outside", { mount: volume.mountPoint })}
          </p>
        )}
      </fieldset>
      <MediaTypeFields
        value={types}
        onChange={(next) => {
          setTypes(next)
          if (add.isError) add.reset()
        }}
        disabled={locked}
        error={noTypes ? t("folders.types.noneSelected") : null}
      />
      {add.isError && (
        <p role="alert" className="text-xs text-destructive break-keep">
          {t(errorKey(add.error))}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <Button type="button" size="sm" variant="outline" disabled={locked} onClick={onLater}>
          {t("volumes.new.later")}
        </Button>
        <Button
          type="button"
          size="sm"
          aria-busy={add.isPending}
          disabled={!canAdd}
          onClick={() => target !== null && add.mutate({ path: target, mediaTypes: types })}
        >
          {t("folders.add.confirm")}
        </Button>
      </div>
    </section>
  )
}
