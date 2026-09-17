/**
 *  EnvVarUpdate.nsh
 *  Updates environment variables (such as PATH) without third-party plugins.
 *  Based on NSIS Wiki EnvVarUpdate function.
 */

!ifndef ENVVARUPDATE_NSH
!define ENVVARUPDATE_NSH

!include "LogicLib.nsh"
!include "WinMessages.nsh"

!macro EnvVarUpdateConstructor UN
Function ${UN}EnvVarUpdate
  Exch $5 ; PathToUpdate
  Exch
  Exch $4 ; RegistryKey ("HKLM" or "HKCU")
  Exch 2
  Exch $3 ; Action ("A" = Append, "P" = Prepend, "R" = Remove)
  Exch 3
  Exch $2 ; VariableName (e.g. "PATH")
  Exch 4
  Exch $1 ; ResultVar
  Push $R0
  Push $R1
  Push $R2
  Push $R3

  ; Determine Root key
  StrCmp $4 "HKLM" 0 +3
    StrCpy $R0 "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"
    Goto done_key
  StrCpy $4 "HKCU"
  StrCpy $R0 "Environment"
done_key:

  ; Read existing variable
  ClearErrors
  ${If} $4 == "HKLM"
    ReadRegStr $R1 HKLM "$R0" "$2"
  ${Else}
    ReadRegStr $R1 HKCU "$R0" "$2"
  ${EndIf}

  ; Check if path already in variable
  Push "$R1"
  Push "$5"
  Call ${UN}StrContains
  Pop $R2

  ${If} $3 == "R"
    ; Remove action
    ${If} $R2 == "1"
      Push "$R1"
      Push ";$5"
      Push ""
      Call ${UN}StrReplace
      Pop $R1

      Push "$R1"
      Push "$5;"
      Push ""
      Call ${UN}StrReplace
      Pop $R1

      Push "$R1"
      Push "$5"
      Push ""
      Call ${UN}StrReplace
      Pop $R1

      ${If} $4 == "HKLM"
        WriteRegExpandStr HKLM "$R0" "$2" "$R1"
      ${Else}
        WriteRegExpandStr HKCU "$R0" "$2" "$R1"
      ${EndIf}
    ${EndIf}
  ${Else}
    ; Add action (Append / Prepend)
    ${If} $R2 == "0"
      ${If} $R1 == ""
        StrCpy $R1 "$5"
      ${Else}
        ${If} $3 == "P"
          StrCpy $R1 "$5;$R1"
        ${Else}
          StrCpy $R1 "$R1;$5"
        ${EndIf}
      ${EndIf}

      ${If} $4 == "HKLM"
        WriteRegExpandStr HKLM "$R0" "$2" "$R1"
      ${Else}
        WriteRegExpandStr HKCU "$R0" "$2" "$R1"
      ${EndIf}
    ${EndIf}
  ${EndIf}

  ; Broadcast WM_SETTINGCHANGE
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000

  Pop $R3
  Pop $R2
  Pop $R1
  Pop $R0
  Pop $1
  Pop $2
  Pop $3
  Pop $4
  Pop $5
FunctionEnd

Function ${UN}StrContains
  Exch $R0 ; SubString
  Exch
  Exch $R1 ; MainString
  Push $R2
  Push $R3
  Push $R4

  StrLen $R2 "$R0"
  StrLen $R3 "$R1"
  StrCpy $R4 0

loop:
  StrCpy $1 "$R1" $R2 $R4
  StrCmp "$1" "$R0" found
  IntOp $R4 $R4 + 1
  IntCmp $R4 $R3 done done loop

found:
  StrCpy $R0 "1"
  Goto end

done:
  StrCpy $R0 "0"

end:
  Pop $R4
  Pop $R3
  Pop $R2
  Pop $R1
  Exch $R0
FunctionEnd

Function ${UN}StrReplace
  Exch $R0 ; ReplaceWith
  Exch
  Exch $R1 ; SearchFor
  Exch 2
  Exch $R2 ; SourceString
  Push $R3
  Push $R4
  Push $R5
  Push $R6

  StrLen $R3 "$R1"
  StrLen $R4 "$R2"
  StrCpy $R5 ""
  StrCpy $R6 0

replace_loop:
  IntCmp $R6 $R4 replace_done replace_done 0
  StrCpy $1 "$R2" $R3 $R6
  StrCmp "$1" "$R1" do_replace

  StrCpy $1 "$R2" 1 $R6
  StrCpy $R5 "$R5$1"
  IntOp $R6 $R6 + 1
  Goto replace_loop

do_replace:
  StrCpy $R5 "$R5$R0"
  IntOp $R6 $R6 + $R3
  Goto replace_loop

replace_done:
  StrCpy $R2 "$R5"
  Pop $R6
  Pop $R5
  Pop $R4
  Pop $R3
  Pop $R1
  Pop $R0
  Exch $R2
FunctionEnd
!macroend

!insertmacro EnvVarUpdateConstructor ""
!insertmacro EnvVarUpdateConstructor "un."

!define EnvVarUpdate '!insertmacro EnvVarUpdateCall'
!macro EnvVarUpdateCall ResultVar Variable Action RegistryKey PathToUpdate
  Push "${ResultVar}"
  Push "${Variable}"
  Push "${Action}"
  Push "${RegistryKey}"
  Push "${PathToUpdate}"
  Call EnvVarUpdate
  Pop "${ResultVar}"
!macroend

!define un.EnvVarUpdate '!insertmacro un.EnvVarUpdateCall'
!macro un.EnvVarUpdateCall ResultVar Variable Action RegistryKey PathToUpdate
  Push "${ResultVar}"
  Push "${Variable}"
  Push "${Action}"
  Push "${RegistryKey}"
  Push "${PathToUpdate}"
  Call un.EnvVarUpdate
  Pop "${ResultVar}"
!macroend

!endif ; ENVVARUPDATE_NSH
