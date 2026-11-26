; Net Lantern's hooks into the Tauri NSIS template (installer.nsi).
;
; The template already gives the Start menu and desktop shortcuts the
; AppUserModelID ${BUNDLEID} (dev.netlantern.app) that toasts need.
;
; An uninstall is either a real one (Settings > Apps, or uninstall.exe opened
; from the install folder) or one the installer drives while replacing the app
; with another version. The driven kind runs the old <install dir>\uninstall.exe in place with `_?=`, so
; $EXEFILE is uninstall.exe; a real uninstall first copies itself to a temp
; folder under another name and runs from there. The Tauri updater also passes
; /UPDATE, which sets $UpdateMode. An unknown name counts as a real uninstall.
;
; A driven uninstall must leave start at login, the data folder and the
; firewall notice alone. The template itself still deletes the Run value then,
; so PREUNINSTALL reads it, POSTUNINSTALL saves it to a file once the uninstall
; has finished, and the new installer's POSTINSTALL puts it back. The file is
; not written earlier because PREUNINSTALL runs before the template's "close
; the running app" prompt, whose Cancel aborts the uninstall. Task Manager's
; StartupApproved switch is never touched on that path.
;
; Keys and value name: netlantern-platform startup.rs RUN_KEY, APPROVED_KEY,
; VALUE_NAME.

; 1 while the installer is driving this uninstall, set once in PREUNINSTALL.
Var NetLanternUpgrade
; The start-at-login command read in PREUNINSTALL, empty when there is none.
Var NetLanternRunCmd

!macro NSIS_HOOK_PREUNINSTALL
  StrCpy $NetLanternUpgrade 0
  ${If} $UpdateMode = 1
  ${OrIf} $EXEFILE == "uninstall.exe"
    StrCpy $NetLanternUpgrade 1
  ${EndIf}

  StrCpy $NetLanternRunCmd ""
  ${If} $NetLanternUpgrade = 1
    ClearErrors
    ReadRegStr $NetLanternRunCmd HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Net Lantern"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Put back the start-at-login value a driven uninstall saved.
  ${If} ${FileExists} "$TEMP\netlantern-upgrade-run.txt"
    Push $0
    Push $1
    ClearErrors
    FileOpen $1 "$TEMP\netlantern-upgrade-run.txt" r
    ${IfNot} ${Errors}
      FileReadUTF16LE $1 $0
      FileClose $1
      ${If} $0 != ""
        WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Net Lantern" $0
      ${EndIf}
    ${EndIf}
    Delete "$TEMP\netlantern-upgrade-run.txt"
    Pop $1
    Pop $0
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $NetLanternUpgrade = 1
    ; The uninstall went through: save the value for the new installer, or
    ; clear any file an earlier upgrade left if start at login is off.
    ${If} $NetLanternRunCmd != ""
      Push $1
      ClearErrors
      FileOpen $1 "$TEMP\netlantern-upgrade-run.txt" w
      ${IfNot} ${Errors}
        FileWriteUTF16LE $1 $NetLanternRunCmd
        FileClose $1
      ${EndIf}
      Pop $1
    ${Else}
      Delete "$TEMP\netlantern-upgrade-run.txt"
    ${EndIf}
  ${EndIf}

  ${If} $NetLanternUpgrade <> 1
    ; Start at login: our own value under Run, and Task Manager's switch for it.
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Net Lantern"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Net Lantern"
    ; A value saved by an upgrade that never finished must not come back later.
    Delete "$TEMP\netlantern-upgrade-run.txt"

    ; The database, its .bak copy and logs\. A custom --data-dir or
    ; NETLANTERN_DATA_DIR folder is left alone.
    ${If} $DeleteAppDataCheckboxState = 1
      SetShellVarContext current
      RMDir /r "$LOCALAPPDATA\NetLantern"
    ${EndIf}

    ; Phone access adds inbound firewall rules as an administrator; a per-user
    ; uninstaller cannot remove them.
    ${IfNot} ${Silent}
      MessageBox MB_OK|MB_ICONINFORMATION "If you turned on phone access, Windows Firewall may still have rules that allow Net Lantern. Removing them needs an administrator: open Windows Security, then Firewall & network protection, Advanced settings, Inbound Rules, and delete the rules for Net Lantern."
    ${EndIf}
  ${EndIf}
!macroend
