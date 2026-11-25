import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import { errorKey, isCommandError } from "@/lib/errors"
import type { EjectPrepared } from "@/lib/ipc-contract"
import { isInsideMount } from "@/lib/volumes"
import { useViewStore } from "@/stores"
import { useInvalidateFolders } from "./useFolderAdmin"
import { qk } from "./keys"

/**
 * 디스크 꺼내기 준비 / 감시 다시 시작 (사이드바 관리 폴더 메뉴).
 * 준비가 끝나면 열려 있는 미디어가 그 디스크 위면 플레이어 패널을 닫아 파일 핸들을 놓는다(`<video>` 경로).
 * 오류는 toast (`invalid_input` 네트워크 폴더, `offline` 이미 끊김 등).
 */
export function useVolumeEject() {
  const api = useApi()
  const qc = useQueryClient()
  const { t } = useTranslation()
  const invalidate = useInvalidateFolders()

  /** 현재 열린 미디어가 `mountPoint` 아래면 패널을 닫는다. 경로를 못 알아내면 닫지 않는다 */
  const closeViewerOn = async (mountPoint: string) => {
    const id = useViewStore.getState().previewId
    if (id === null) return
    try {
      const detail = await qc.ensureQueryData({ queryKey: qk.asset(id), queryFn: () => api.getAssetDetail(id) })
      if (useViewStore.getState().previewId === id && isInsideMount(detail.path, mountPoint)) useViewStore.getState().closePreview()
    } catch {
      // 상세를 못 읽으면 어떤 디스크인지 알 수 없다 — 준비 자체는 성공이므로 패널은 그대로 둔다
    }
  }

  const prepare = useMutation({
    mutationFn: (folderId: number) => api.prepareVolumeEject(folderId),
    onSuccess: async (res: EjectPrepared) => {
      await closeViewerOn(res.mountPoint)
      invalidate()
      void qc.invalidateQueries({ queryKey: qk.assets })
      toast.success(t("folders.eject.prepared", { mount: res.mountPoint }))
    },
    // 공용 errors.busy 는 "폴더 제거 중" 문구라 맞지 않는다 — 꺼내기 전용 문구를 쓴다
    onError: (e) => void toast.error(t(isCommandError(e) && e.code === "busy" ? "folders.eject.busy" : errorKey(e))),
  })

  const resume = useMutation({
    mutationFn: (folderId: number) => api.resumeVolumeWatch(folderId),
    onSuccess: () => {
      invalidate()
      void qc.invalidateQueries({ queryKey: qk.assets })
      toast.success(t("folders.eject.resumed"))
    },
    onError: (e) => void toast.error(t(errorKey(e))),
  })

  return { prepare, resume }
}
