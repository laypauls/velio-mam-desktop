param(
  [int]$ProcessId,
  [ValidateSet("dump", "open", "toggle", "quit", "hide")] [string]$Action = "dump"
)
# Drives the Windows notification-area icon of the given process with UI Automation + a real right click.
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public class Win {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
}
"@
$AE = [System.Windows.Automation.AutomationElement]
$TS = [System.Windows.Automation.TreeScope]
$root = $AE::RootElement

function Find-ByName($scopeEl, $pattern) {
  $all = $scopeEl.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
  foreach ($e in $all) { if ($e.Current.Name -like $pattern) { return $e } }
  return $null
}
function Click-At($x, $y, $right) {
  [Win]::SetCursorPos([int]$x, [int]$y) | Out-Null
  Start-Sleep -Milliseconds 150
  if ($right) { [Win]::mouse_event(0x08, 0, 0, 0, [UIntPtr]::Zero); [Win]::mouse_event(0x10, 0, 0, 0, [UIntPtr]::Zero) }
  else { [Win]::mouse_event(0x02, 0, 0, 0, [UIntPtr]::Zero); [Win]::mouse_event(0x04, 0, 0, 0, [UIntPtr]::Zero) }
}
function Center($el) { $r = $el.Current.BoundingRectangle; return @(($r.X + $r.Width / 2), ($r.Y + $r.Height / 2)) }

$proc = Get-Process -Id $ProcessId
$hwnd = $proc.MainWindowHandle

if ($Action -eq "hide") { [Win]::ShowWindow($hwnd, 0) | Out-Null; Start-Sleep -Milliseconds 500; "visible=" + [Win]::IsWindowVisible($hwnd); exit 0 }

# 1) tray icon: it lives in the overflow flyout ("show hidden icons" chevron = first SystemTray.NormalButton in the taskbar)
function Find-TrayIcon() {
  foreach ($e in $root.FindAll($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($AE::ClassNameProperty, "SystemTray.NormalButton")))) {
    if ($e.Current.Name -like "velio.*") { return $e }
  }
  return $null
}
$icon = Find-TrayIcon
if (-not $icon) {
  $taskbar = $root.FindFirst($TS::Children, (New-Object System.Windows.Automation.PropertyCondition($AE::ClassNameProperty, "Shell_TrayWnd")))
  $chev = $taskbar.FindFirst($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($AE::ClassNameProperty, "SystemTray.NormalButton")))
  $c = Center $chev; Click-At $c[0] $c[1] $false; Start-Sleep -Milliseconds 1000
  $icon = Find-TrayIcon
}
if (-not $icon) { "TRAY ICON NOT FOUND"; exit 2 }
"icon: name='" + $icon.Current.Name + "' rect=" + $icon.Current.BoundingRectangle

# 2) real right click -> native popup menu
$c = Center $icon
Click-At $c[0] $c[1] $true
Start-Sleep -Milliseconds 800
$menu = $null
foreach ($w in $root.FindAll($TS::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
  if ($w.Current.ClassName -eq "#32768") { $menu = $w; break }
}
if (-not $menu) { "MENU NOT FOUND"; exit 3 }
# The native popup menu exposes no UIA children, so click by geometry (fractions measured from a screenshot of the real menu:
# row 1 = open, row 2 = pause/resume, separator, row 3 = quit). A screenshot of the menu is saved for the record.
$r = $menu.Current.BoundingRectangle
Add-Type -AssemblyName System.Drawing
$bmp = New-Object System.Drawing.Bitmap ([int]$r.Width + 100), ([int]$r.Height + 100)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen([int]($r.X - 50), [int]($r.Y - 50), 0, 0, $bmp.Size)
$shot = Join-Path $PSScriptRoot ("results\menu-" + $Action + ".png")
$bmp.Save($shot)
"menu rect=" + $r
if ($Action -eq "dump") { [System.Windows.Forms.SendKeys]::SendWait("{ESC}"); exit 0 }
$frac = switch ($Action) { "open" { 0.17 } "toggle" { 0.43 } "quit" { 0.82 } }
$x = $r.X + $r.Width / 2; $y = $r.Y + $r.Height * $frac
"clicking row at " + [int]$x + "," + [int]$y
Click-At $x $y $false
Start-Sleep -Milliseconds 800
$alive = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
"process alive=" + [bool]$alive
if ($alive) { "mainWindowVisible=" + [Win]::IsWindowVisible($hwnd) }
