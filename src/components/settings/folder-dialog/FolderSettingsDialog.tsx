import { useEffect, type FormEvent, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { useChangeDetectors } from "@/hooks/useFolderDetection"
import { useFolders } from "@/hooks/useLibrary"
import { useSettings } from "@/hooks/useSettings"
import type { ManagedFolder } from "@/lib/ipc-contract"
import { POLICY_UNKNOWN } from "@/lib/rescan-policy"
import { CopyCompleteSection } from "../detection/CopyCompleteSection"
import { RescanPolicySection } from "../detection/RescanPolicySection"
import { WatchModeSection } from "../detection/WatchModeSection"
import { AliasSection } from "./AliasSection"
import { FolderActionsSection, FolderPathRow } from "./FolderActionsSection"
import { MediaTypesSection } from "./MediaTypesSection"
import { useFolderSettingsSave } from "./useFolderSettingsSave"

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-3">
      <h3 className="text-sm font-semibold">{title}</h3>
      {children}
    </section>
  )
}

function SettingsForm({ folder, onClose }: { folder: ManagedFolder; onClose: () => void }) {
  const { t } = useTranslation()
  const settings = useSettings()
  const defaultHours = settings.data?.rescanIntervalHours ?? 6
  const detectors = useChangeDetectors(folder.folderId)
  const form = useFolderSettingsSave(folder, detectors.data, defaultHours)
  const axisError = (axis: string) => (form.error?.axis === axis ? form.error.message : null)
  const detectionError = axisError("policy") ?? axisError("watch")

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (await form.save()) onClose()
  }

  return (
    <form onSubmit={(e) => void submit(e)} className="flex min-h-0 flex-col gap-6">
      {/* 본문만 스크롤 — relative 로 내부 absolute/sr-only 자손이 바깥 조상 기준으로 배치되는 것을 막는다 */}
      <div className="relative -mx-1 flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto px-1">
        <Section title={t("settings.folders.dialog.sectionGeneral")}>
          <AliasSection value={form.alias.value} onChange={form.alias.onChange} tooLong={form.alias.tooLong} error={axisError("alias")} />
          <FolderPathRow folder={folder} dirty={form.dirty} />
        </Section>
        <Section title={t("settings.folders.dialog.sectionTypes")}>
          <MediaTypesSection
            folderId={folder.folderId}
            value={form.types.value}
            onChange={form.types.onChange}
            excluded={form.types.excluded}
            mode={form.types.mode}
            onModeChange={form.types.onModeChange}
            disabled={form.busy}
            error={form.types.none ? t("folders.types.noneSelected") : axisError("types")}
          />
        </Section>
        <Section title={t("settings.folders.dialog.sectionDetection")}>
          <WatchModeSection
            folder={folder}
            detectors={detectors.data}
            mode={form.watch.draft.mode}
            onModeChange={form.watch.onModeChange}
            optionValues={form.watch.draft.options}
            optionErrors={form.watch.optionErrors}
            onOptionChange={form.watch.onOptionChange}
          />
          <CopyCompleteSection exclusive={form.watch.draft.exclusive} disabled={form.busy} onExclusiveChange={form.watch.onExclusiveChange} />
          <RescanPolicySection
            folder={folder}
            draft={form.policy.draft}
            storedUnknown={form.policy.base.kind === POLICY_UNKNOWN}
            errors={form.policy.errors}
            defaultHours={defaultHours}
            noAutoWarning={form.policy.noAutoWarning}
            draftChanged={form.policy.changed}
            onChange={form.policy.onChange}
          />
          {detectionError !== null && (
            <p role="alert" className="text-xs text-destructive break-keep">
              {detectionError}
            </p>
          )}
        </Section>
        <Section title={t("settings.folders.dialog.sectionActions")}>
          <FolderActionsSection folder={folder} dirty={form.dirty} />
        </Section>
      </div>
      <DialogFooter>
        <Button type="button" variant="outline" disabled={form.busy} onClick={onClose}>
          {t("common.cancel")}
        </Button>
        <Button type="submit" aria-busy={form.busy} disabled={!form.dirty || form.invalid || form.busy}>
          {t("common.save")}
        </Button>
      </DialogFooter>
    </form>
  )
}

interface Props {
  /** 열 폴더 (null = 닫힘) — 경로 변경·저장 뒤 목록 갱신을 따라가도록 id 로 받아 최신 폴더를 읽는다 */
  folderId: number | null
  onClose: () => void
}

/** 관리 폴더 1개의 별칭·스캔 대상·변경 감지·재스캔 일정 편집과 폴더 변경·재스캔·제거를 한 Dialog 에서 한다. */
export function FolderSettingsDialog({ folderId, onClose }: Props) {
  const { t } = useTranslation()
  const folders = useFolders()
  const folder = folderId === null ? null : (folders.data?.find((f) => f.folderId === folderId) ?? null)
  const gone = folderId !== null && folders.data !== undefined && folder === null

  // 폴더가 제거되면(이 다이얼로그에서든 다른 곳에서든) 함께 닫는다
  useEffect(() => {
    if (gone) onClose()
  }, [gone, onClose])

  return (
    <Dialog open={folderId !== null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] grid-rows-[auto_minmax(0,1fr)] sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("settings.folders.dialog.title")}</DialogTitle>
          <DialogDescription className="break-keep">{t("settings.folders.dialog.description")}</DialogDescription>
        </DialogHeader>
        {/* key 로 대상 폴더마다 입력·오류 상태를 새로 시작한다 */}
        {folder && <SettingsForm key={folder.folderId} folder={folder} onClose={onClose} />}
      </DialogContent>
    </Dialog>
  )
}
