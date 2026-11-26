; Net Lantern's hooks into the Tauri NSIS template (installer.nsi).
;
; The template already gives the Start menu and desktop shortcuts the
; AppUserModelID ${BUNDLEID} (dev.netlantern.app) that toasts need, so there is
; no post-install hook.
;
; The uninstaller runs with /UPDATE when an update replaces the app, and the
; template sets $UpdateMode to 1 then. Nothing here runs on an update: start at
; login, the data folder and the firewall notice all belong to a real uninstall.

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    ; Start at login: our own value under Run, and Task Manager's switch for it
    ; (netlantern-platform startup.rs: RUN_KEY, APPROVED_KEY, VALUE_NAME).
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Net Lantern"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Net Lantern"

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
