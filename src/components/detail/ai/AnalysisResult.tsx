import { Pencil } from "lucide-react"
import { useId, useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { useUpdateAnalysis } from "@/hooks/useAi"
import { errorKey } from "@/lib/errors"
import type { AiAnalysis } from "@/lib/ipc-contract-ai"

function Field({ title, children, action }: { title: string; children: ReactNode; action?: ReactNode }) {
  return (
    <section className="flex flex-col gap-1">
      <div className="flex items-center gap-2">
        <h3 className="text-xs font-medium text-muted-foreground">{title}</h3>
        {action}
      </div>
      {children}
    </section>
  )
}

function EditForm({ assetId, analysis, onDone }: { assetId: number; analysis: AiAnalysis; onDone: () => void }) {
  const { t } = useTranslation()
  const update = useUpdateAnalysis(assetId)
  const errorId = useId()
  const [description, setDescription] = useState(analysis.description)
  const [keywords, setKeywords] = useState(analysis.keywords.join(", "))
  const [error, setError] = useState<string | null>(null)

  const save = async () => {
    setError(null)
    const list = keywords
      .split(",")
      .map((k) => k.trim())
      .filter(Boolean)
    const edit = {
      ...(description !== analysis.description ? { description } : {}),
      ...(list.join("\n") !== analysis.keywords.join("\n") ? { keywords: list } : {}),
    }
    if (Object.keys(edit).length === 0) return onDone()
    try {
      await update.mutateAsync(edit)
      onDone()
    } catch (e) {
      setError(t(errorKey(e)))
    }
  }

  return (
    <div className="flex flex-col gap-2">
      <Textarea
        rows={4}
        value={description}
        aria-label={t("ai.result.description")}
        aria-invalid={error !== null}
        aria-describedby={error !== null ? errorId : undefined}
        onChange={(e) => setDescription(e.target.value)}
      />
      <Input
        value={keywords}
        aria-label={t("ai.result.keywordsEdit")}
        aria-invalid={error !== null}
        aria-describedby={error !== null ? errorId : undefined}
        onChange={(e) => setKeywords(e.target.value)}
      />
      {error && (
        <p id={errorId} role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
      <div className="flex gap-2">
        <Button size="sm" disabled={update.isPending} onClick={() => void save()}>
          {t("common.save")}
        </Button>
        <Button size="sm" variant="outline" onClick={onDone}>
          {t("common.cancel")}
        </Button>
      </div>
    </div>
  )
}

/** D4: 설명·키워드·OCR·요약. 설명과 키워드는 편집할 수 있고, 편집하면 "편집됨" 을 표시한다. */
export function AnalysisResult({ assetId, analysis }: { assetId: number; analysis: AiAnalysis }) {
  const { t } = useTranslation()
  const [editing, setEditing] = useState(false)
  return (
    <div className="flex flex-col gap-3">
      <Field
        title={t("ai.result.description")}
        action={
          <>
            {analysis.edited && (
              <Badge variant="secondary" className="rounded-sm">
                {t("ai.result.edited")}
              </Badge>
            )}
            {!editing && (
              <Button
                size="icon-xs"
                variant="ghost"
                className="ml-auto"
                aria-label={t("ai.result.edit")}
                onClick={() => setEditing(true)}
              >
                <Pencil aria-hidden="true" />
              </Button>
            )}
          </>
        }
      >
        {editing ? (
          <EditForm assetId={assetId} analysis={analysis} onDone={() => setEditing(false)} />
        ) : (
          <>
            {analysis.description && <p className="text-sm break-words select-text">{analysis.description}</p>}
            {analysis.keywords.length > 0 && (
              <ul aria-label={t("ai.result.keywords")} className="flex flex-wrap gap-1">
                {analysis.keywords.map((k) => (
                  <li key={k}>
                    <Badge variant="outline" className="rounded-sm select-text">
                      {k}
                    </Badge>
                  </li>
                ))}
              </ul>
            )}
          </>
        )}
      </Field>
      {analysis.ocrText && (
        <Field title={t("ai.result.ocr")}>
          <p className="text-sm break-words select-text">{analysis.ocrText}</p>
        </Field>
      )}
      {analysis.summary && (
        <Field title={t("ai.result.summary")}>
          <p className="text-sm break-words select-text">{analysis.summary}</p>
        </Field>
      )}
    </div>
  )
}
