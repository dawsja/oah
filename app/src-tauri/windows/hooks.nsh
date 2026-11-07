; oah installer hooks.
;
; Uninstalling oah (not updating it) deletes the speech-to-text model it downloaded for voice
; (731 MB, Settings > Voice). Chats, settings and keychain entries stay unless "Delete the
; application data" is ticked, which Tauri handles for its own folders only.

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    RMDir /r "$PROFILE\.config\oah\app\speech"
  ${EndIf}
!macroend
