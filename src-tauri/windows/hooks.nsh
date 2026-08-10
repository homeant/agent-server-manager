!include "LogicLib.nsh"
!include "StrFunc.nsh"
!include "WinMessages.nsh"

${StrStr}
${StrRep}
${un.StrRep}

; The desktop installer owns the GUI, while the headless CLI sidecar is installed next to it.
; Add that directory to the current user's PATH so a fresh terminal can run `asvc` directly.
!macro NSIS_HOOK_POSTINSTALL
  ReadRegStr $0 HKCU "Environment" "Path"
  ${StrStr} $1 $0 "$INSTDIR"
  ${If} $1 == ""
    StrCmp $0 "" 0 +2
      StrCpy $0 "$INSTDIR"
    StrCmp $0 "$INSTDIR" +2 0
      StrCpy $0 "$INSTDIR;$0"
    WriteRegExpandStr HKCU "Environment" "Path" $0
    SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ReadRegStr $0 HKCU "Environment" "Path"
  ${If} $0 == "$INSTDIR"
    StrCpy $1 ""
  ${Else}
    ${un.StrRep} $1 $0 "$INSTDIR;" ""
    ${If} $1 == $0
      ${un.StrRep} $1 $0 ";$INSTDIR" ""
    ${EndIf}
  ${EndIf}
  WriteRegExpandStr HKCU "Environment" "Path" $1
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
!macroend
