import { useState } from "react"
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
import { Button } from "@/components/ui/button"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { SCENE_SENSITIVITY_OPTIONS } from "@/components/viewer/SceneRedetectDialog"
import { useSceneBackfill, useSceneBackfillCount } from "@/hooks/useSceneCuts"
import { useUpdateSettings } from "@/hooks/useSettings"
import { formatNumber } from "@/lib/format"
import type { SceneSensitivity, Settings } from "@/lib/ipc-contract"
import { SettingRow } from "./SettingRow"
import { SettingsSubsection } from "./SettingsSubsection"

/**
 * 설정 > 플레이어 > 장면전환점: 탐색 바 표시, 새 동영상 자동 추출(끔이 기본 — CPU 를 많이 쓴다), 검출 민감도, 기존 동영상 모두 추출(확인 필요). 단축키 안내는 "단축키" 소제목.
 * 민감도를 바꿔도 이미 추출된 영상은 자동으로 다시 추출하지 않는다 (영상별 "다시 추출…" 로만).
 */
export function SceneCutsSection({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  const backfill = useSceneBackfill()
  const count = useSceneBackfillCount().data ?? 0
  const [confirmOpen, setConfirmOpen] = useState(false)
  return (
    <SettingsSubsection title={t("settings.scene.title")}>
      <SettingRow label={t("settings.scene.visible")} description={t("settings.scene.visibleHint")}>
        <Switch aria-label={t("settings.scene.visible")} checked={settings.sceneCutsVisible} onCheckedChange={(sceneCutsVisible) => update.mutate({ sceneCutsVisible })} />
      </SettingRow>
      <SettingRow label={t("settings.scene.auto")} description={t("settings.scene.autoHint")}>
        <Switch aria-label={t("settings.scene.auto")} checked={settings.sceneDetectAuto} onCheckedChange={(sceneDetectAuto) => update.mutate({ sceneDetectAuto })} />
      </SettingRow>
      <SettingRow label={t("scene.sensitivity")} description={t("settings.scene.sensitivityHint")}>
        <Select value={settings.sceneDetectSensitivity} onValueChange={(v) => update.mutate({ sceneDetectSensitivity: v as SceneSensitivity })}>
          <SelectTrigger aria-label={t("scene.sensitivity")} className="w-40">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {SCENE_SENSITIVITY_OPTIONS.map((s) => (
              <SelectItem key={s} value={s}>
                {t(`scene.sensitivity_${s}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </SettingRow>
      <SettingRow label={t("settings.scene.existing")} description={t("settings.scene.existingHint")}>
        <Button variant="outline" disabled={count === 0 || backfill.isPending} onClick={() => setConfirmOpen(true)}>
          {t("settings.scene.backfill", { count: formatNumber(count) })}
        </Button>
      </SettingRow>
      <AlertDialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.scene.confirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription className="break-keep">{t("settings.scene.confirmDescription", { count: formatNumber(count) })}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => backfill.mutate()}>{t("settings.scene.confirm")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </SettingsSubsection>
  )
}
