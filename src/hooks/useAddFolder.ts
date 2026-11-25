import { useMutation } from "@tanstack/react-query"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { useAddFolderRequest } from "./useFolderAdmin"

/**
 * 관리 폴더 추가 흐름: 폴더 선택 → (대상 유형 Dialog) → add_folder. 4개 진입점 공용.
 * 선택 취소는 아무것도 하지 않는다. 폴더 선택 실패는 toast, add_folder 실패는 Dialog 안 인라인(`add.error`).
 * 호출부는 `start()` 로 시작하고 `<AddFolderDialog flow={…} />` 를 한 번 렌더한다.
 */
export function useAddFolder() {
  const api = useApi()
  const { t } = useTranslation()
  const [path, setPath] = useState<string | null>(null)
  const add = useAddFolderRequest()
  const pick = useMutation({
    mutationFn: () => api.pickDirectory(),
    onSuccess: (dir) => {
      if (dir !== null) setPath(dir)
    },
    onError: (e) => void toast.error(t(errorKey(e))),
  })
  return {
    path,
    add,
    isPending: pick.isPending,
    start: () => pick.mutate(),
    close: () => {
      setPath(null)
      add.reset()
    },
  }
}

export type AddFolderFlow = ReturnType<typeof useAddFolder>
