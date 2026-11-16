; Windows can't replace a program that is running. Termlab's background service keeps
; running after the window closes, so it is stopped before files are installed or removed.
; Its sessions end with it; the ones that can be resumed come back when Termlab starts.

!macro NSIS_HOOK_PREINSTALL
  nsExec::Exec 'taskkill /F /IM termlabd.exe'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'taskkill /F /IM termlabd.exe'
  Pop $0
!macroend
