import type { ReactNode } from "react"
import { DropdownMenu as DropdownMenuPrimitive } from "radix-ui"
import { useTranslation } from "react-i18next"
import {
  CARD_SIZES,
  CARD_SIZE_KEYS,
  INFO_KEYS,
  PRESETS,
  presetOf,
  type CardInfoKey,
  type CardPresetName,
} from "@/components/gallery/cardDisplay"
import {
  DropdownMenuCheckboxItem,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuSeparator,
} from "@/components/ui/dropdown-menu"
import type { CardSize, GalleryCardDisplay } from "@/lib/ipc-contract"
import { useViewStore } from "@/stores"

/** 카드 표시 메뉴 본문을 담는 컨테이너 클래스 — 항목이 많아 세로 여백을 이 메뉴에서만 줄인다 (전역 shadcn 스타일은 그대로) */
export const CARD_MENU_CONTENT_CLASS = "w-80 max-w-[calc(100vw-1rem)] [&_[data-slot=dropdown-menu-label]]:py-1 [&_[role^=menuitem]]:py-1"

/** 항목을 골라도 메뉴가 닫히지 않게 한다 — 여러 옵션을 연달아 바꾸고 카드 변화를 바로 본다 */
const keepOpen = (e: Event) => e.preventDefault()

function Hint({ children }: { children: ReactNode }) {
  return <span className="ml-auto text-xs text-muted-foreground">{children}</span>
}

/**
 * 한 줄 선택 — 왼쪽 라벨, 오른쪽 분할 버튼 모양의 라디오 그룹. 키보드·접근성을 위해 버튼이 아니라 메뉴 라디오 항목을 그대로 쓴다
 * (메뉴 안에서는 Tab 이 막혀 있고 화살표는 메뉴 항목 사이만 이동한다). 공용 ui 컴포넌트는 인디케이터·좌측 여백이 있어 Radix 원형을 직접 쓴다.
 */
function SegmentRow({
  label,
  value,
  onValueChange,
  options,
}: {
  label: string
  value: string
  onValueChange: (v: string) => void
  options: { value: string; text: string; title?: string }[]
}) {
  return (
    <div className="flex items-center justify-between gap-2 px-2 py-1">
      <span className="text-sm break-keep">{label}</span>
      <DropdownMenuRadioGroup
        aria-label={label}
        value={value}
        onValueChange={onValueChange}
        className="inline-flex shrink-0 overflow-hidden rounded-md border border-border"
      >
        {options.map((o) => (
          <DropdownMenuPrimitive.RadioItem
            key={o.value}
            aria-label={o.title}
            title={o.title}
            value={o.value}
            onSelect={keepOpen}
            className="inline-flex h-7 cursor-default items-center justify-center border-l border-border px-2 text-xs break-keep outline-hidden select-none first:border-l-0 data-[highlighted]:bg-muted data-[state=checked]:bg-accent data-[state=checked]:font-medium"
          >
            {o.text}
          </DropdownMenuPrimitive.RadioItem>
        ))}
      </DropdownMenuRadioGroup>
    </div>
  )
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <>
      <DropdownMenuSeparator />
      <DropdownMenuLabel className="text-xs text-muted-foreground">{title}</DropdownMenuLabel>
      {children}
    </>
  )
}

const PRESET_NAMES = Object.keys(PRESETS) as CardPresetName[]
const CHECK_MODES = ["hover", "always", "hidden"] as const
const CHECK_KEY = { hover: "checkHover", always: "checkAlways", hidden: "checkHidden" } as const
const DATE_BASES = ["captured", "modified"] as const
const FLAGS = ["favorite", "rating", "duration", "quickActions"] as const


/** 카드 표시 옵션 항목들 — `CardDisplayMenu`(격자 툴바)와 상단 바 "더보기" 하위 메뉴가 같이 쓴다. 호출 측이 메뉴 컨테이너(Content/SubContent)를 제공한다. */
export function CardDisplayItems() {
  const { t } = useTranslation()
  const display = useViewStore((s) => s.cardDisplay)
  const setCardDisplay = useViewStore((s) => s.setCardDisplay)
  const setCardSize = useViewStore((s) => s.setCardSize)
  const applyCardPreset = useViewStore((s) => s.applyCardPreset)
  const resetCardDisplay = useViewStore((s) => s.resetCardDisplay)
  const preset = presetOf(display)

  return (
    <>
      <DropdownMenuLabel className="flex items-center justify-between gap-2">
        {t("cardDisplay.title")}
        <span className="text-xs font-normal text-muted-foreground">
          {preset === "custom" ? t("cardDisplay.custom") : t(`cardDisplay.preset.${preset}`)}
        </span>
      </DropdownMenuLabel>
      <SegmentRow
        label={t("cardDisplay.size.label")}
        value={display.cardSize}
        onValueChange={(v) => setCardSize(v as CardSize)}
        options={CARD_SIZE_KEYS.map((k) => ({
          value: k,
          text: t(`cardDisplay.size.${k}`),
          title: t("cardDisplay.size.option", { name: t(`cardDisplay.size.${k}`), px: CARD_SIZES[k] }),
        }))}
      />
      <DropdownMenuSeparator />
      <SegmentRow
        label={t("cardDisplay.preset.label")}
        value={preset}
        onValueChange={(v) => applyCardPreset(v as CardPresetName)}
        options={PRESET_NAMES.map((n) => ({ value: n, text: t(`cardDisplay.preset.${n}`) }))}
      />

      <Section title={t("cardDisplay.info.label")}>
        {INFO_KEYS.map((k: CardInfoKey) => (
          <DropdownMenuCheckboxItem
            key={k}
            checked={display.info[k]}
            onCheckedChange={(v) => setCardDisplay({ info: { [k]: v === true } })}
            onSelect={keepOpen}
          >
            {t(`cardDisplay.info.${k}`)}
            {k === "resolution" && <Hint>{t("cardDisplay.info.resolutionHint")}</Hint>}
          </DropdownMenuCheckboxItem>
        ))}
        <SegmentRow
          label={t("cardDisplay.dateBy.label")}
          value={display.dateBy}
          onValueChange={(v) => setCardDisplay({ dateBy: v as GalleryCardDisplay["dateBy"] })}
          options={DATE_BASES.map((d) => ({ value: d, text: t(`cardDisplay.dateBy.${d}`) }))}
        />
      </Section>

      <Section title={t("cardDisplay.overlay.label")}>
        <SegmentRow
          label={t("cardDisplay.overlay.check")}
          value={display.check}
          onValueChange={(v) => setCardDisplay({ check: v as GalleryCardDisplay["check"] })}
          options={CHECK_MODES.map((m) => ({ value: m, text: t(`cardDisplay.overlay.${CHECK_KEY[m]}`) }))}
        />
        {FLAGS.map((f) => (
          <DropdownMenuCheckboxItem
            key={f}
            checked={display[f]}
            onCheckedChange={(v) => setCardDisplay({ [f]: v === true })}
            onSelect={keepOpen}
          >
            {t(`cardDisplay.overlay.${f}`)}
          </DropdownMenuCheckboxItem>
        ))}
        <DropdownMenuCheckboxItem checked disabled>
          {t("cardDisplay.overlay.status")}
          <Hint>{t("cardDisplay.overlay.statusLocked")}</Hint>
        </DropdownMenuCheckboxItem>
      </Section>

      <DropdownMenuSeparator />
      <DropdownMenuItem onSelect={(e) => {
          keepOpen(e)
          resetCardDisplay()
        }} className="justify-end">
        {t("cardDisplay.reset")}
      </DropdownMenuItem>
    </>
  )
}
