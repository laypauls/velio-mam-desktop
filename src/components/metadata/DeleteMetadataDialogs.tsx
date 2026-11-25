import { Loader2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { useDeleteMetadataField, useDeleteMetadataOption, useMetadataFieldValueCount, useMetadataOptionUsage } from "@/hooks/useMetadataFields"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import type { MetadataField, MetadataFieldOption } from "@/lib/ipc-contract-metadata"

interface BodyProps {
  /** i18n 키 접두 — `metadata.settings.deleteField` / `deleteOption` */
  ns: string
  name: string
  count: { isPending: boolean; isError: boolean; data: number | undefined }
  mutation: { isPending: boolean; isError: boolean; error: unknown; mutate: (opts: { onSuccess: () => void }) => void }
  onClose: () => void
}

function DeleteBody({ ns, name, count, mutation, onClose }: BodyProps) {
  const { t } = useTranslation()
  return (
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>{t(`${ns}.title`)}</AlertDialogTitle>
        <AlertDialogDescription className="break-all">{t(`${ns}.description`, { name })}</AlertDialogDescription>
      </AlertDialogHeader>
      <div className="flex flex-col gap-2 text-sm break-keep">
        {count.isPending ? (
          <p role="status" aria-busy="true" className="flex items-center gap-1.5 text-muted-foreground">
            <Loader2 aria-hidden="true" className="size-4 animate-spin" />
            {t(`${ns}.loading`)}
          </p>
        ) : count.isError ? (
          <p className="text-muted-foreground">{t(`${ns}.countError`)}</p>
        ) : (
          <p role="status" className="font-medium text-destructive">
            {count.data === 0 ? t(`${ns}.impactNone`) : t(`${ns}.impact`, { count: formatNumber(count.data ?? 0) })}
          </p>
        )}
        {mutation.isError && (
          <p role="alert" className="text-destructive">
            {t(errorKey(mutation.error))}
          </p>
        )}
      </div>
      <AlertDialogFooter>
        <AlertDialogCancel disabled={mutation.isPending}>{t("common.cancel")}</AlertDialogCancel>
        <AlertDialogAction
          variant="destructive"
          aria-busy={mutation.isPending}
          disabled={mutation.isPending || count.isPending}
          onClick={(e) => {
            // 오류가 나면 창을 열어 둔 채 인라인으로 보여 주고, 성공했을 때만 닫는다
            e.preventDefault()
            mutation.mutate({ onSuccess: onClose })
          }}
        >
          {t("metadata.settings.delete")}
        </AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  )
}

/** 필드 삭제 확인 — 값 n건이 함께 삭제되며 복원할 수 없음을 알린다. 성공하면 목록에서 바로 사라진다. */
export function DeleteFieldDialog({ field, onClose }: { field: MetadataField | null; onClose: () => void }) {
  const count = useMetadataFieldValueCount(field?.fieldId ?? null)
  const del = useDeleteMetadataField()
  return (
    <AlertDialog open={field !== null} onOpenChange={(o) => !o && !del.isPending && onClose()}>
      {field && (
        <DeleteBody
          key={field.fieldId}
          ns="metadata.settings.deleteField"
          name={field.name}
          count={count}
          mutation={{ ...del, mutate: (o) => del.mutate(field.fieldId, o) }}
          onClose={onClose}
        />
      )}
    </AlertDialog>
  )
}

/** 선택지 삭제 확인 — 사용 중 n건의 값이 지워짐을 알린다. */
export function DeleteOptionDialog({ option, onClose }: { option: MetadataFieldOption | null; onClose: () => void }) {
  const usage = useMetadataOptionUsage(option?.optionId ?? null)
  const del = useDeleteMetadataOption()
  return (
    <AlertDialog open={option !== null} onOpenChange={(o) => !o && !del.isPending && onClose()}>
      {option && (
        <DeleteBody
          key={option.optionId}
          ns="metadata.settings.deleteOption"
          name={option.label}
          count={usage}
          mutation={{ ...del, mutate: (o) => del.mutate(option.optionId, o) }}
          onClose={onClose}
        />
      )}
    </AlertDialog>
  )
}
