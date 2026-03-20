; Kokoro Engine NSIS hooks
; Prevent app data (memories, conversations, settings) from being deleted
; during uninstall or upgrade.

!macro NSIS_HOOK_PREUNINSTALL
  ; Force the "Delete app data" checkbox state to unchecked (0)
  ; so $APPDATA\com.chyin.kokoro is never removed.
  StrCpy $DeleteAppDataCheckboxState 0
!macroend
