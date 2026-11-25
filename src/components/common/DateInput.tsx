import { CalendarDays } from "lucide-react"
import { useState, type ComponentProps, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Calendar } from "@/components/ui/calendar"
import { Input } from "@/components/ui/input"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { cn } from "@/lib/utils"
import { calendarLocale, formatDateText, parseDateText } from "./dateText"

type InputProps = Omit<ComponentProps<"input">, "type" | "value" | "onChange" | "className" | "placeholder">

interface Props extends InputProps {
  /** `YYYY-MM-DD` 텍스트 (입력 중에는 불완전한 문자열일 수 있다) */
  value: string
  /** 직접 입력(`typed`) 또는 달력·지우기·오늘 선택(`picked`) 때 호출 */
  onValueChange: (text: string, source: "typed" | "picked") => void
  /** 바깥 래퍼에 적용 (폭 등) */
  className?: string
}

/**
 * 날짜 입력 — 텍스트 칸(`YYYY-MM-DD`) + 달력 팝오버.
 * 브라우저 기본 `type="date"` 는 WebView2 의 UI 언어를 따라 앱 표시 언어와 어긋나므로 쓰지 않는다.
 */
export function DateInput({ value, onValueChange, className, ...inputProps }: Props): ReactNode {
  const { t, i18n } = useTranslation()
  const [open, setOpen] = useState(false)
  const selected = parseDateText(value)

  const pick = (text: string) => {
    onValueChange(text, "picked")
    setOpen(false)
  }

  return (
    <div className={cn("relative", className)}>
      <Input
        {...inputProps}
        type="text"
        inputMode="numeric"
        autoComplete="off"
        placeholder={t("dateInput.placeholder")}
        value={value}
        className="w-full pr-8"
        onChange={(e) => onValueChange(e.target.value, "typed")}
      />
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={t("dateInput.openCalendar")}
            className="absolute top-1/2 right-0.5 size-7 -translate-y-1/2"
          >
            <CalendarDays aria-hidden="true" />
          </Button>
        </PopoverTrigger>
        <PopoverContent align="end" className="w-auto gap-1 p-1">
          <Calendar
            mode="single"
            required
            selected={selected}
            defaultMonth={selected ?? new Date()}
            locale={calendarLocale(i18n.resolvedLanguage)}
            onSelect={(d) => pick(formatDateText(d))}
          />
          <div className="flex justify-between px-2 pb-1">
            <Button type="button" variant="ghost" size="sm" onClick={() => pick("")}>
              {t("dateInput.clear")}
            </Button>
            <Button type="button" variant="ghost" size="sm" onClick={() => pick(formatDateText(new Date()))}>
              {t("dateInput.today")}
            </Button>
          </div>
        </PopoverContent>
      </Popover>
    </div>
  )
}
