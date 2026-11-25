import { useEffect } from "react"
import { useUpdater } from "@/hooks/useUpdater"
import { UpdateDialog } from "./UpdateDialog"

/** 시작 후 `startupDelayMs` 뒤 1회 자동 확인(실패는 조용히 무시) + 업데이트 Dialog. */
export function UpdateManager({ startupDelayMs }: { startupDelayMs: number }) {
  const { check } = useUpdater()
  useEffect(() => {
    const id = setTimeout(() => void check(false), startupDelayMs)
    return () => clearTimeout(id)
  }, [check, startupDelayMs])
  return <UpdateDialog />
}
