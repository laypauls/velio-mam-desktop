import { BulkAnalyzeDialog } from "@/components/ai/BulkAnalyzeDialog"
import { ConsentDialog } from "@/components/ai/ConsentDialog"
import { OnboardingDialog } from "@/components/protect/OnboardingDialog"
import { OriginalsDialog } from "@/components/protect/OriginalsDialog"
import { PasswordGuideDialog } from "@/components/protect/PasswordGuideDialog"
import { PasswordPromptDialog } from "@/components/protect/PasswordPromptDialog"
import { ResetDialog } from "@/components/protect/ResetDialog"
import { BulkTagDialog } from "./BulkTagDialog"
import { CollectionFormDialog } from "./CollectionFormDialog"
import { DeleteCollectionDialog } from "./DeleteCollectionDialog"
import { ExportDialog } from "./ExportDialog"
import { TagManagerDialog } from "./TagManagerDialog"

/** 앱 어디서나 열 수 있는 작업 Dialog 모음 (열림 상태는 `useActionsStore` / `useProtectStore`). */
export function ActionDialogs() {
  return (
    <>
      <BulkTagDialog />
      <CollectionFormDialog />
      <DeleteCollectionDialog />
      <ExportDialog />
      <TagManagerDialog />
      <OriginalsDialog />
      <PasswordPromptDialog />
      <PasswordGuideDialog />
      <ResetDialog />
      <OnboardingDialog />
      <ConsentDialog />
      <BulkAnalyzeDialog />
    </>
  )
}
