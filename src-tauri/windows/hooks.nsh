; NSIS installer hooks (P1-7 update rollback). Wired by bundle.windows.nsis.installerHooks.
;
; Contract with src-tauri/src/update_guard (paths live under MAMData\updates, see Paths there):
;   pending-setup.exe     this installer, copied after install; the app promotes it to
;                         current-setup.exe once the new version reached "healthy"
;   current-setup.exe     installer of the last healthy version (the rollback target)
;   update-in-progress    written by the running app just before an update; only then do we verify
;   rollback-requested    written here when verification failed; tells the restored app to restore the DB
;
; MAMData location, same order as the app (mam-core storage::resolve_location):
;   1. MAM_DATA_DIR (env, tests/diagnostics)
;   2. %LOCALAPPDATA%\MAMDesktop\data-dir.txt - mirror of the location the app is actually using
;      (location.json is JSON, which NSIS cannot parse). The app writes it as UTF-16LE with BOM, one line, no
;      newline; FileReadUTF16LE skips the BOM (verified with NSIS 3 Unicode).
;   3. default %LOCALAPPDATA%\MAMDesktop\MAMData
; Uninstall never touches MAMData (it is not under the install dir nor the bundle-identifier dirs).

; Brand display name "velio." (D-3). productName is "velio" (no period: a install folder name cannot end in '.'),
; so the Tauri template writes shortcuts "velio.lnk" and DisplayName "velio". We rename the shortcuts to
; "velio..lnk" (shown as "velio." - Windows hides the .lnk extension) and overwrite DisplayName.
; Template order (Section Install): registry writes -> Start Menu shortcut -> desktop shortcut (silent/passive only)
; -> NSIS_HOOK_POSTINSTALL. The interactive desktop shortcut is created later by the finish page, see .onGUIEnd below.
; Update mode (/UPDATE) and /NS skip shortcut creation, so an existing "velio..lnk" is left as the user has it.
!define VELIO_LNK "velio..lnk"

; Rename "<dir>\<PRODUCTNAME>.lnk" -> "<dir>\velio..lnk" only when it points at our exe (never touch foreign shortcuts)
!macro VELIO_RENAME_LNK dir
  ${If} ${FileExists} "${dir}\${PRODUCTNAME}.lnk"
    !insertmacro IsShortcutTarget "${dir}\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      Delete "${dir}\${VELIO_LNK}"
      Rename "${dir}\${PRODUCTNAME}.lnk" "${dir}\${VELIO_LNK}"
    ${EndIf}
  ${EndIf}
!macroend

; Remove "<dir>\velio..lnk" (and its pin) only when it points at our exe
!macro VELIO_DELETE_LNK dir
  ${If} ${FileExists} "${dir}\${VELIO_LNK}"
    !insertmacro IsShortcutTarget "${dir}\${VELIO_LNK}" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      !insertmacro UnpinShortcut "${dir}\${VELIO_LNK}"
      Delete "${dir}\${VELIO_LNK}"
    ${EndIf}
  ${EndIf}
!macroend

; Interactive install: the finish-page checkbox creates "$DESKTOP\velio.lnk" AFTER the install section, so rename it
; when the GUI closes. Literal names on purpose: this file is included before PRODUCTNAME/MAINBINARYNAME are defined.
Function .onGUIEnd
  ${If} ${FileExists} "$DESKTOP\velio.lnk"
    !insertmacro IsShortcutTarget "$DESKTOP\velio.lnk" "$INSTDIR\velio.exe"
    Pop $0
    ${If} $0 = 1
      Delete "$DESKTOP\velio..lnk"
      Rename "$DESKTOP\velio.lnk" "$DESKTOP\velio..lnk"
    ${EndIf}
  ${EndIf}
FunctionEnd

!macro NSIS_HOOK_POSTINSTALL
  ; velio. display name: shortcuts + uninstall list entry (see header)
  !insertmacro VELIO_RENAME_LNK "$SMPROGRAMS"
  !insertmacro VELIO_RENAME_LNK "$DESKTOP"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "velio."

  ; Language packs: copy "$INSTDIR\locales\*.json" -> "%LOCALAPPDATA%\MAMDesktop\locales" (fixed folder, independent of
  ; MAM_DATA_DIR), only files that do not exist yet - never overwrite user-edited packs. The app also seeds at startup
  ; (src-tauri/src/locale/seed.rs) for `tauri dev` and deleted files; both paths copy missing files only.
  ; Runs before the update check so passive auto-update installs get new packs too. Uses $R7-$R9 (free of $R0-$R6 below).
  CreateDirectory "$LOCALAPPDATA\MAMDesktop\locales"
  FindFirst $R7 $R8 "$INSTDIR\locales\*.json"
  ${DoWhile} $R8 != ""
    StrCpy $R9 "$LOCALAPPDATA\MAMDesktop\locales\$R8"
    ${IfNot} ${FileExists} "$R9"
      CopyFiles /SILENT "$INSTDIR\locales\$R8" "$R9"
    ${EndIf}
    FindNext $R7 $R8
  ${Loop}
  FindClose $R7

  ReadEnvStr $R0 "MAM_DATA_DIR"
  ${If} $R0 == ""
    StrCpy $R0 "$LOCALAPPDATA\MAMDesktop\MAMData"
    ${If} ${FileExists} "$LOCALAPPDATA\MAMDesktop\data-dir.txt"
      FileOpen $R5 "$LOCALAPPDATA\MAMDesktop\data-dir.txt" r
      FileReadUTF16LE $R5 $R6
      FileClose $R5
      ${If} $R6 != ""
        StrCpy $R0 $R6
      ${EndIf}
    ${EndIf}
  ${EndIf}
  StrCpy $R1 "$R0\updates"
  CreateDirectory "$R1"

  ; keep this installer so a later update can roll back to this version
  CopyFiles /SILENT "$EXEPATH" "$R1\pending-setup.exe"

  ${If} ${FileExists} "$R1\update-in-progress"
    ; post-install check: open DB + migrate with the new exe, no UI, exit code decides
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --verify-update' $R2
    ${If} $R2 <> 0
      Delete "$R1\update-in-progress"
      FileOpen $R3 "$R1\rollback-requested" w
      FileClose $R3
      ${If} ${FileExists} "$R1\current-setup.exe"
        ; silent reinstall of the previous version over this one; the restored app restores the DB
        ExecWait '"$R1\current-setup.exe" /S' $R4
      ${EndIf}
    ${EndIf}
  ${EndIf}
!macroend

; The template only removes "${PRODUCTNAME}.lnk"; remove our renamed shortcuts (not during an update)
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    !insertmacro VELIO_DELETE_LNK "$SMPROGRAMS"
    !insertmacro VELIO_DELETE_LNK "$DESKTOP"
  ${EndIf}
!macroend
