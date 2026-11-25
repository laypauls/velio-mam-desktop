import type { ComponentType, ReactNode } from "react"
import { useTranslation } from "react-i18next"
import {
  ContextMenuCheckboxItem,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
} from "@/components/ui/context-menu"
import {
  DropdownMenuCheckboxItem,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
} from "@/components/ui/dropdown-menu"
import { MEMBERSHIP_MAX_IDS, useCollectionMembership, useToggleMembership } from "@/hooks/useCollectionOrder"
import { useCollections } from "@/hooks/useLibrary"
import { formatNumber } from "@/lib/format"
import { useActionsStore } from "@/stores"

interface CheckItemProps {
  checked: boolean | "indeterminate"
  disabled?: boolean
  onSelect: (e: Event) => void
  "aria-busy"?: boolean
  children: ReactNode
}

/** 우클릭 메뉴와 드롭다운 메뉴가 같은 목록을 그리도록, 각 메뉴의 항목 컴포넌트를 주입한다. */
export interface MenuPrimitives {
  CheckboxItem: ComponentType<CheckItemProps>
  Item: ComponentType<{ onSelect: (e: Event) => void; children: ReactNode }>
  Separator: ComponentType
  Label: ComponentType<{ children: ReactNode }>
}

export const CONTEXT_MENU_PRIMITIVES: MenuPrimitives = {
  CheckboxItem: ContextMenuCheckboxItem,
  Item: ContextMenuItem,
  Separator: ContextMenuSeparator,
  Label: ContextMenuLabel,
}

export const DROPDOWN_MENU_PRIMITIVES: MenuPrimitives = {
  CheckboxItem: DropdownMenuCheckboxItem,
  Item: DropdownMenuItem,
  Separator: DropdownMenuSeparator,
  Label: DropdownMenuLabel,
}

/**
 * "컬렉션에 추가" 메뉴 본문 — 선택 자산이 이미 속한 컬렉션을 체크로 보여 준다.
 * 모두 속하면 ✓, 일부면 －(indeterminate), 없으면 빈칸. 클릭 = ✓ 이면 제거, 그 외는 추가하고 메뉴는 열어 둔다.
 * 메뉴가 열릴 때만 마운트되므로 소속 조회도 열려 있는 동안만 일어난다.
 */
export function CollectionCheckItems({ ids, primitives: P }: { ids: readonly number[]; primitives: MenuPrimitives }) {
  const { t } = useTranslation()
  const collections = useCollections()
  const membership = useCollectionMembership(ids, true)
  const toggle = useToggleMembership(ids)
  const openForm = useActionsStore((s) => s.openCollectionForm)
  const tooMany = ids.length > MEMBERSHIP_MAX_IDS
  const loading = !tooMany && membership.isPending
  const counts = new Map((membership.data ?? []).map((m) => [m.collectionId, m.count]))
  const list = collections.data ?? []

  return (
    <>
      <P.Label>{t("collections.membership.selected", { count: formatNumber(ids.length) })}</P.Label>
      {tooMany && <P.Label>{t("collections.membership.tooMany", { max: formatNumber(MEMBERSHIP_MAX_IDS) })}</P.Label>}
      {list.map((c) => {
        const n = counts.get(c.collectionId) ?? 0
        // 조회 중에는 체크 자리를 비워 두고(aria-busy) 누를 수 없게 한다 — 잘못된 추가/제거 방지
        const checked = loading ? false : n >= ids.length && n > 0 ? true : n > 0 ? "indeterminate" : false
        return (
          <P.CheckboxItem
            key={c.collectionId}
            checked={checked}
            disabled={tooMany || loading}
            aria-busy={loading || undefined}
            onSelect={(e) => {
              e.preventDefault()
              toggle(c.collectionId, checked === true)
            }}
          >
            <span className="min-w-0 flex-1 truncate">{c.name}</span>
            <span className="text-xs text-muted-foreground tabular-nums">{formatNumber(c.assetCount)}</span>
          </P.CheckboxItem>
        )
      })}
      {list.length > 0 && <P.Separator />}
      <P.Item onSelect={() => openForm({ mode: "create", assetIds: [...ids] })}>{t("collections.new")}</P.Item>
    </>
  )
}
