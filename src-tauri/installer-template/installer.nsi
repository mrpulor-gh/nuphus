Unicode true
ManifestDPIAware true
; Add in `dpiAwareness` `PerMonitorV2` to manifest for Windows 10 1607+ (note this should not affect lower versions since they should be able to ignore this and pick up `dpiAware` `true` set by `ManifestDPIAware true`)
; Currently undocumented on NSIS's website but is in the Docs folder of source tree, see
; https://github.com/kichik/nsis/blob/5fc0b87b819a9eec006df4967d08e522ddd651c9/Docs/src/attributes.but#L286-L300
; https://github.com/tauri-apps/tauri/pull/10106
ManifestDPIAwareness PerMonitorV2

!if "{{compression}}" == "none"
  SetCompress off
!else
  ; Set the compression algorithm. We default to LZMA.
  SetCompressor /SOLID "{{compression}}"
!endif

; Keep above !include to stay ahead of any plugin command
; see https://github.com/tauri-apps/tauri/pull/15422#discussion_r3289239624
{{#if signed_plugins_path}}
!addplugindir "{{signed_plugins_path}}"
{{/if}}

!include MUI2.nsh
!include FileFunc.nsh
!include x64.nsh
!include WordFunc.nsh
!include "utils.nsh"
!include "FileAssociation.nsh"
!include "Win\COM.nsh"
!include "Win\Propkey.nsh"
!include "Win\RestartManager.nsh"
!include "StrFunc.nsh"
${StrCase}
${StrLoc}
${StrStr}

; StrFunc.nsh **没有** StrContains（2026-10-31 makensis 实测：
; `Invalid command: "${StrContains}"` → 整个安装包编译中止）。
; 依赖检测/下载失败归因只需要「子串是否出现」：用 StrStr 包一层，
; 大小写不敏感（先把 haystack 转小写），返回非空 = 命中；
; 旧调用只做 `!= ""` 判断，不依赖位置数值，语义等价。
!macro NuphusStrContains out haystack needle
  ${StrCase} ${out} "${haystack}" "L"
  ${StrStr} ${out} "${out}" "${needle}"
!macroend
!define StrContains `!insertmacro NuphusStrContains`

{{#if installer_hooks}}
!include "{{installer_hooks}}"
{{/if}}

!define WEBVIEW2APPGUID "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

!define MANUFACTURER "{{manufacturer}}"
!define PRODUCTNAME "{{product_name}}"
!define VERSION "{{version}}"
!define VERSIONWITHBUILD "{{version_with_build}}"
!define HOMEPAGE "{{homepage}}"
!define INSTALLMODE "{{install_mode}}"
!define LICENSE "{{license}}"
!define INSTALLERICON "{{installer_icon}}"
!define SIDEBARIMAGE "{{sidebar_image}}"
!define HEADERIMAGE "{{header_image}}"
!define UNINSTALLERICON "{{uninstaller_icon}}"
!define UNINSTALLERHEADERIMAGE "{{uninstaller_header_image}}"
!define MAINBINARYNAME "{{main_binary_name}}"
!define MAINBINARYSRCPATH "{{main_binary_path}}"
!define BUNDLEID "{{bundle_id}}"
!define COPYRIGHT "{{copyright}}"
!define OUTFILE "{{out_file}}"
!define ARCH "{{arch}}"
!define ADDITIONALPLUGINSPATH "{{additional_plugins_path}}"
!define ALLOWDOWNGRADES "{{allow_downgrades}}"
!define DISPLAYLANGUAGESELECTOR "{{display_language_selector}}"
!define INSTALLWEBVIEW2MODE "{{install_webview2_mode}}"
!define WEBVIEW2INSTALLERARGS "{{webview2_installer_args}}"
!define WEBVIEW2BOOTSTRAPPERPATH "{{webview2_bootstrapper_path}}"
!define WEBVIEW2INSTALLERPATH "{{webview2_installer_path}}"
!define MINIMUMWEBVIEW2VERSION "{{minimum_webview2_version}}"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"
!define MANUKEY "Software\${MANUFACTURER}"
!define MANUPRODUCTKEY "${MANUKEY}\${PRODUCTNAME}"
!define UNINSTALLERSIGNCOMMAND "{{uninstaller_sign_cmd}}"
!define ESTIMATEDSIZE "{{estimated_size}}"
!define STARTMENUFOLDER "{{start_menu_folder}}"

Var PassiveMode
Var UpdateMode
Var NoShortcutMode
Var WixMode
Var OldMainBinaryName

Name "${PRODUCTNAME}"
BrandingText "${COPYRIGHT}"
OutFile "${OUTFILE}"

; We don't actually use this value as default install path,
; it's just for nsis to append the product name folder in the directory selector
; https://nsis.sourceforge.io/Reference/InstallDir
!define PLACEHOLDER_INSTALL_DIR "placeholder\${PRODUCTNAME}"
InstallDir "${PLACEHOLDER_INSTALL_DIR}"

VIProductVersion "${VERSIONWITHBUILD}"
VIAddVersionKey "ProductName" "${PRODUCTNAME}"
VIAddVersionKey "FileDescription" "${PRODUCTNAME}"
VIAddVersionKey "LegalCopyright" "${COPYRIGHT}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

# additional plugins
!addplugindir "${ADDITIONALPLUGINSPATH}"

; Uninstaller signing command
!if "${UNINSTALLERSIGNCOMMAND}" != ""
  !uninstfinalize '${UNINSTALLERSIGNCOMMAND}'
!endif

; Handle install mode, `perUser`, `perMachine` or `both`
!if "${INSTALLMODE}" == "perMachine"
  RequestExecutionLevel admin
!endif

!if "${INSTALLMODE}" == "currentUser"
  RequestExecutionLevel user
!endif

!if "${INSTALLMODE}" == "both"
  !define MULTIUSER_MUI
  !define MULTIUSER_INSTALLMODE_INSTDIR "${PRODUCTNAME}"
  !define MULTIUSER_INSTALLMODE_COMMANDLINE
  !if "${ARCH}" == "x64"
    !define MULTIUSER_USE_PROGRAMFILES64
  !else if "${ARCH}" == "arm64"
    !define MULTIUSER_USE_PROGRAMFILES64
  !endif
  !define MULTIUSER_INSTALLMODE_DEFAULT_REGISTRY_KEY "${UNINSTKEY}"
  !define MULTIUSER_INSTALLMODE_DEFAULT_REGISTRY_VALUENAME "CurrentUser"
  !define MULTIUSER_INSTALLMODEPAGE_SHOWUSERNAME
  !define MULTIUSER_INSTALLMODE_FUNCTION RestorePreviousInstallLocation
  !define MULTIUSER_EXECUTIONLEVEL Highest
  !include MultiUser.nsh
!endif

; Installer icon
!if "${INSTALLERICON}" != ""
  !define MUI_ICON "${INSTALLERICON}"
!endif

; Installer sidebar image
!if "${SIDEBARIMAGE}" != ""
  !define MUI_WELCOMEFINISHPAGE_BITMAP "${SIDEBARIMAGE}"
!endif

; Enable header images for installer and uninstaller pages when either image is configured.
!if "${HEADERIMAGE}" != ""
  !define MUI_HEADERIMAGE
!else if "${UNINSTALLERHEADERIMAGE}" != ""
  !define MUI_HEADERIMAGE
!endif

; Installer header image
!if "${HEADERIMAGE}" != ""
  !define MUI_HEADERIMAGE_BITMAP "${HEADERIMAGE}"
!endif

; Uninstaller header image
!if "${UNINSTALLERHEADERIMAGE}" != ""
  !define MUI_HEADERIMAGE_UNBITMAP "${UNINSTALLERHEADERIMAGE}"
!endif

; Uninstaller icon
!if "${UNINSTALLERICON}" != ""
  !define MUI_UNICON "${UNINSTALLERICON}"
!endif

; Define registry key to store installer language
!define MUI_LANGDLL_REGISTRY_ROOT "HKCU"
!define MUI_LANGDLL_REGISTRY_KEY "${MANUPRODUCTKEY}"
!define MUI_LANGDLL_REGISTRY_VALUENAME "Installer Language"

; Installer pages, must be ordered as they appear
;
; ── 安装向导（2026-10-31 简化，大王验收口径）──────────────────────────
; 目标流程：打开安装程序 → 选安装目录 → 安装 → 完成。
; 已移除的用户交互页：
;   · 欢迎页      —— 没有信息量，直接进目录选择；
;   · 依赖检测页  —— 只是「告知」，检测与自动下载仍在安装 Section 内执行
;                    （WebView2 / VC++ 运行库缺失才下载，不占交互步骤）；
;   · 维护/重装页 —— 已安装时直接覆盖升级（沿用上次安装目录），不再让用户
;                    选「卸载后再装 / 不卸载」；
;   · 完成页的「创建桌面快捷方式」勾选 —— 安装钩子默认创建，勾选框是重复决策。
; 保留：目录页（用户唯一需要做的选择）、安装进度页、完成页（运行 Nuphus）。
; passive/pin 更新模式仍由 SkipIfPassive 自动跳过目录页。

; 1. Choose install directory page（唯一交互页）
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_DIRECTORY

; 2. License Page (if defined)
!if "${LICENSE}" != ""
  !define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
  !insertmacro MUI_PAGE_LICENSE "${LICENSE}"
!endif

; 3. Install mode (if it is set to `both`)
!if "${INSTALLMODE}" == "both"
  !define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
  !insertmacro MULTIUSER_PAGE_INSTALLMODE
!endif

; 4. 维护/重装页已移除（2026-10-31 简化）：已安装时直接覆盖升级。
;    相关函数（PageReinstall/PageLeaveReinstall/FindWixInstall）保留但不再挂页，
;    避免影响沉默安装/更新路径之外的历史逻辑引用。
Var ReinstallPageCheck

; Find a previous WiX installation and set $R6 to its uninstall registry key, or to "" if none.
;
; A WiX installer stores the installation info in registry
; using a UUID and so we have to loop through all keys under
; `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall`
; and check if `DisplayName` and `Publisher` keys match ${PRODUCTNAME} and ${MANUFACTURER}
;
; This has a potential issue that there maybe another installation that matches
; our ${PRODUCTNAME} and ${MANUFACTURER} but wasn't installed by our WiX installer,
; however, this should be fine since the user will have to confirm the uninstallation
; and they can chose to abort it if doesn't make sense.
Function FindWixInstall
  StrCpy $R6 ""
  StrCpy $0 0
  wix_loop:
    EnumRegKey $1 HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall" $0
    StrCmp $1 "" wix_loop_done ; Exit loop if there is no more keys to loop on
    IntOp $0 $0 + 1
    ReadRegStr $R0 HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$1" "DisplayName"
    ReadRegStr $R1 HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$1" "Publisher"
    StrCmp "$R0$R1" "${PRODUCTNAME}${MANUFACTURER}" 0 wix_loop
    ReadRegStr $R0 HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$1" "UninstallString"
    ${StrCase} $R1 $R0 "L"
    ${StrLoc} $R0 $R1 "msiexec" ">"
    StrCmp $R0 0 0 wix_loop_done
    StrCpy $R6 "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$1"
  wix_loop_done:
FunctionEnd

Function PageReinstall
  ; Uninstall previous WiX installation if exists.
  Call FindWixInstall
  ${If} $R6 != ""
    StrCpy $WixMode 1
    Goto compare_version
  ${EndIf}

  ; Check if there is an existing installation, if not, abort the reinstall page
  ReadRegStr $R0 SHCTX "${UNINSTKEY}" ""
  ReadRegStr $R1 SHCTX "${UNINSTKEY}" "UninstallString"
  ${IfThen} "$R0$R1" == "" ${|} Abort ${|}

  ; Compare this installar version with the existing installation
  ; and modify the messages presented to the user accordingly
  compare_version:
  StrCpy $R4 "$(older)"
  ${If} $WixMode = 1
    ReadRegStr $R0 HKLM "$R6" "DisplayVersion"
  ${Else}
    ReadRegStr $R0 SHCTX "${UNINSTKEY}" "DisplayVersion"
  ${EndIf}
  ${IfThen} $R0 == "" ${|} StrCpy $R4 "$(unknown)" ${|}

  nsis_tauri_utils::SemverCompare "${VERSION}" $R0
  Pop $R0

  ; Read the install choice header through $R5 so a custom language file that only
  ; defines the misspelled `choowHowToInstall` still shows its own text
  ; TODO: remove the `choowHowToInstall` fallback in a future major version
  StrCpy $R5 "$(chooseHowToInstall)"
  ${IfThen} $R5 == "" ${|} StrCpy $R5 "$(choowHowToInstall)" ${|}

  ; Reinstalling the same version
  ${If} $R0 = 0
    StrCpy $R1 "$(alreadyInstalledLong)"
    StrCpy $R2 "$(addOrReinstall)"
    StrCpy $R3 "$(uninstallApp)"
    !insertmacro MUI_HEADER_TEXT "$(alreadyInstalled)" "$(chooseMaintenanceOption)"
  ; Upgrading
  ${ElseIf} $R0 = 1
    StrCpy $R1 "$(olderOrUnknownVersionInstalled)"
    StrCpy $R2 "$(uninstallBeforeInstalling)"
    StrCpy $R3 "$(dontUninstall)"
    !insertmacro MUI_HEADER_TEXT "$(alreadyInstalled)" "$R5"
  ; Downgrading
  ${ElseIf} $R0 = -1
    StrCpy $R1 "$(newerVersionInstalled)"
    StrCpy $R2 "$(uninstallBeforeInstalling)"
    !if "${ALLOWDOWNGRADES}" == "true"
      StrCpy $R3 "$(dontUninstall)"
    !else
      StrCpy $R3 "$(dontUninstallDowngrade)"
    !endif
    !insertmacro MUI_HEADER_TEXT "$(alreadyInstalled)" "$R5"
  ${Else}
    Abort
  ${EndIf}

  ; Skip showing the page if passive
  ;
  ; Note that we don't call this earlier at the beginning
  ; of this function because we need to populate some variables
  ; related to current installed version if detected and whether
  ; we are downgrading or not.
  ${If} $PassiveMode = 1
    Call PageLeaveReinstall
  ${Else}
    nsDialogs::Create 1018
    Pop $R4
    ${IfThen} $(^RTL) = 1 ${|} nsDialogs::SetRTL $(^RTL) ${|}

    ${NSD_CreateLabel} 0 0 100% 24u $R1
    Pop $R1

    ${NSD_CreateRadioButton} 30u 50u -30u 8u $R2
    Pop $R2
    ${NSD_OnClick} $R2 PageReinstallUpdateSelection

    ${NSD_CreateRadioButton} 30u 70u -30u 8u $R3
    Pop $R3
    ; Disable this radio button if downgrading and downgrades are disabled
    !if "${ALLOWDOWNGRADES}" == "false"
      ${IfThen} $R0 = -1 ${|} EnableWindow $R3 0 ${|}
    !endif
    ${NSD_OnClick} $R3 PageReinstallUpdateSelection

    ; Check the first radio button if this the first time
    ; we enter this page or if the second button wasn't
    ; selected the last time we were on this page
    ${If} $ReinstallPageCheck <> 2
      SendMessage $R2 ${BM_SETCHECK} ${BST_CHECKED} 0
    ${Else}
      SendMessage $R3 ${BM_SETCHECK} ${BST_CHECKED} 0
    ${EndIf}

    ${NSD_SetFocus} $R2
    nsDialogs::Show
  ${EndIf}
FunctionEnd
Function PageReinstallUpdateSelection
  ${NSD_GetState} $R2 $R1
  ${If} $R1 == ${BST_CHECKED}
    StrCpy $ReinstallPageCheck 1
  ${Else}
    StrCpy $ReinstallPageCheck 2
  ${EndIf}
FunctionEnd
Function PageLeaveReinstall
  ${NSD_GetState} $R2 $R1

  ; If migrating from Wix, always uninstall
  ${If} $WixMode = 1
    Goto reinst_uninstall
  ${EndIf}

  ; In update mode, always proceeds without uninstalling
  ${If} $UpdateMode = 1
    Goto reinst_done
  ${EndIf}

  ; $R0 holds whether same(0)/upgrading(1)/downgrading(-1) version
  ; $R1 holds the radio buttons state:
  ;   1 => first choice was selected
  ;   0 => second choice was selected
  ${If} $R0 = 0 ; Same version, proceed
    ${If} $R1 = 1              ; User chose to add/reinstall
      Goto reinst_done
    ${Else}                    ; User chose to uninstall
      Goto reinst_uninstall
    ${EndIf}
  ${ElseIf} $R0 = 1 ; Upgrading
    ${If} $R1 = 1              ; User chose to uninstall
      Goto reinst_uninstall
    ${Else}
      Goto reinst_done         ; User chose NOT to uninstall
    ${EndIf}
  ${ElseIf} $R0 = -1 ; Downgrading
    ${If} $R1 = 1              ; User chose to uninstall
      Goto reinst_uninstall
    ${Else}
      Goto reinst_done         ; User chose NOT to uninstall
    ${EndIf}
  ${EndIf}

  reinst_uninstall:
    HideWindow
    ClearErrors

    ${If} $WixMode = 1
      ReadRegStr $R1 HKLM "$R6" "UninstallString"
      ExecWait '$R1' $0
    ${Else}
      ReadRegStr $4 SHCTX "${MANUPRODUCTKEY}" ""
      ReadRegStr $R1 SHCTX "${UNINSTKEY}" "UninstallString"
      ${IfThen} $UpdateMode = 1 ${|} StrCpy $R1 "$R1 /UPDATE" ${|} ; append /UPDATE
      ${IfThen} $PassiveMode = 1 ${|} StrCpy $R1 "$R1 /P" ${|} ; append /P
      StrCpy $R1 "$R1 _?=$4" ; append uninstall directory
      ExecWait '$R1' $0
    ${EndIf}

    BringToFront

    ${IfThen} ${Errors} ${|} StrCpy $0 2 ${|} ; ExecWait failed, set fake exit code

    ${If} $0 <> 0
    ${OrIf} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
      ; User cancelled wix uninstaller? return to select un/reinstall page
      ${If} $WixMode = 1
      ${AndIf} $0 = 1602
        Abort
      ${EndIf}

      ; User cancelled NSIS uninstaller? return to select un/reinstall page
      ${If} $0 = 1
        Abort
      ${EndIf}

      ; Other errors? show generic error message and return to select un/reinstall page
      MessageBox MB_ICONEXCLAMATION "$(unableToUninstall)"
      Abort
    ${EndIf}

    ; The NSIS uninstaller was run in place (`_?=`), so it cannot delete itself
    ; and the install directory, clean them up here once the uninstall succeeded
    ; https://nsis.sourceforge.io/Docs/Chapter3.html#:~:text=path%20contains%20spaces.-,3.2.3%20Examples,-installer.exe%20/NCRC
    ${If} $WixMode = 0
    ${AndIf} $4 != ""
      Delete "$4\uninstall.exe"
      RMDir "$4"
    ${EndIf}
  reinst_done:
FunctionEnd

; ═══════════════════════════════════════════════════════════════════════════
; Nuphus 自定义:运行时依赖检测函数群(2026-10-30)
;
; 设计原则(大王验收口径):
;   * 检测先行 —— 必要依赖(WebView2 / VC++ 运行库)在欢迎页后即如实展示状态;
;   * 按需下载 —— 缺了才下(官方源),不让所有用户承担离线包的体积;
;   * 失败明示 —— 任何失败都给出人话原因 + 原始错误 + 手动获取地址,可重试。
; ═══════════════════════════════════════════════════════════════════════════

; 读取当前机器的 WebView2 运行库版本(官方检测口径)
; 输出:$4 = 版本串(pv 注册表值);"" = 未安装(含 0.0.0.0 占位值)
Function ReadWebview2Pv
  ${If} ${RunningX64}
    ReadRegStr $4 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${Else}
    ReadRegStr $4 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${EndIf}
  ${If} $4 == ""
    ReadRegStr $4 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${EndIf}
  ; 微软文档:pv 为 0.0.0.0 等同于未安装
  ${If} $4 == "0.0.0.0"
    StrCpy $4 ""
  ${EndIf}
FunctionEnd

; 检测 VC++ 2015-2022 (v14) x64 运行库是否已安装
; 输出:$R9 = 0(已安装)/ 1(缺失);$R8 = 失败时的说明位(预留)
; 检测口径(微软官方文档 recommended):注册表表
;   HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64 的 Installed=1
; 通过 /REGVIEW64 读 64 位注册表视图(本安装器为 32 位 NSIS,
; 64 位 DLL 在 System32、注册表在 64 位视图,直接读会被 WOW64 重定向走偏)。
Function DetectVcRuntime
  StrCpy $R9 1
  ; ReadRegStr 不支持 /REGVIEW64 开关（2026-10-31 makensis 实测：
  ; `ReadRegStr expects 4 parameters, got 5`）——用 SetRegView 切视图，
  ; 读完必须切回 32 位默认视图，避免影响后续注册表读写。
  ${If} ${RunningX64}
    SetRegView 64
    ReadRegStr $R6 HKLM "SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64" "Installed"
    SetRegView 32
  ${Else}
    ReadRegStr $R6 HKLM "SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x86" "Installed"
  ${EndIf}
  ${If} $R6 == "1"
    StrCpy $R9 0
  ${EndIf}
FunctionEnd

; 依赖检测页(自定义 nsDialogs 页)
Function PageDependencies
  ; passive(应用内更新)/ silent / 更新模式:无页面交互,跳过(下载仍在
  ; 安装 Section 内自动进行)
  ${If} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
  ${OrIf} ${Silent}
    Abort
  ${EndIf}

  ; 先探测状态(两行寄存器),组件齐全则整页跳过,不浪费用户注意力
  Call ReadWebview2Pv
  StrCpy $R1 $4
  Call DetectVcRuntime
  ${If} $R1 != ""
  ${AndIf} $R9 = 0
    Abort
  ${EndIf}

  !insertmacro MUI_HEADER_TEXT "$(depCheckHeader)" "$(depCheckSub)"

  nsDialogs::Create 1018
  Pop $0
  ${IfThen} $(^RTL) = 1 ${|} nsDialogs::SetRTL $(^RTL) ${|}

  ; WebView2 行
  ${If} $R1 == ""
    StrCpy $R2 "$(depWebview2Missing)"
    StrCpy $R4 "$(depWebview2StatusMiss)"
  ${Else}
    StrCpy $R2 "$(depWebview2Ok)"
    StrCpy $R4 "$(depWebview2StatusOk)"
  ${EndIf}
  ; VC++ 行
  ${If} $R9 = 1
    StrCpy $R3 "$(depVcMissing)"
    StrCpy $R5 "$(depVcStatusMiss)"
  ${Else}
    StrCpy $R3 "$(depVcOk)"
    StrCpy $R5 "$(depVcStatusOk)"
  ${EndIf}

  ${NSD_CreateLabel} 0 5u 100% 12u "$(depCheckLead)"
  Pop $R7
  ${NSD_CreateLabel} 8u 25u 8u 10u "$R4"
  Pop $R7
  ${NSD_CreateLabel} 18u 25u -20u 14u "$R2"
  Pop $R7
  ${NSD_CreateLabel} 8u 45u 8u 10u "$R5"
  Pop $R7
  ${NSD_CreateLabel} 18u 45u -20u 14u "$R3"
  Pop $R7
  ${NSD_CreateLabel} 0 78u 100% 32u "$(depNote)"
  Pop $R7

  nsDialogs::Show
FunctionEnd

; 离开页回调:本页无输入,空实现(与模板 Reinstall 页同构)
Function PageLeaveDependencies
FunctionEnd

; NSISdl 下载失败的原因人话化(WebView2 与 VC++ 下载共用)
; 输入:$0 = NSISdl 结果串; $R7 = 组件主题名(np装配进提示)
; 输出:$1 = 完整提示文本(MessageBox 用,含原始错误码串)
Function DescribeNsisdlDownloadFailure
  ; 依据 NSISdl 的结果串做关键词归因(官方文档仅保证 "success"/"cancel"
  ; 为固定值,其余为自由描述 —— 所以归因失败时一律落到通用网络受限并原样
  ; 携带原始串,不猜造原因)
  StrCpy $2 "$(depReasonGeneric)"
  ${StrContains} $R8 $0 "cancel"
  ${If} $R8 != ""
    StrCpy $2 "$(depReasonCancel)"
  ${EndIf}
  ${StrContains} $R8 $0 "connection"
  ${If} $R8 != ""
    StrCpy $2 "$(depReasonConnect)"
  ${EndIf}
  ${StrContains} $R8 $0 "timeout"
  ${If} $R8 != ""
    StrCpy $2 "$(depReasonTimeout)"
  ${EndIf}
  ${StrContains} $R8 $0 "resolve"
  ${If} $R8 != ""
    StrCpy $2 "$(depReasonResolve)"
  ${EndIf}
  ${StrContains} $R8 $0 "open"
  ${If} $R8 != ""
    StrCpy $2 "$(depReasonWrite)"
  ${EndIf}
  StrCpy $1 "$R7$(depFailureTitle)$(depReasonBody)$2$\n$\n$(depRawPrefix)$0$\n$\n$(depRetryGuide)"
FunctionEnd

; ─ 5. 目录页已在上方（唯一交互页）声明，此处不再重复插入
; 6. Start menu shortcut page
Var AppStartMenuFolder
!if "${STARTMENUFOLDER}" != ""
  !define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
  !define MUI_STARTMENUPAGE_DEFAULTFOLDER "${STARTMENUFOLDER}"
!else
  !define MUI_PAGE_CUSTOMFUNCTION_PRE Skip
!endif
!insertmacro MUI_PAGE_STARTMENU Application $AppStartMenuFolder

; 7. Installation page
!insertmacro MUI_PAGE_INSTFILES

; 8. Finish page
;
; Don't auto jump to finish page after installation page,
; because the installation page has useful info that can be used debug any issues with the installer.
!define MUI_FINISHPAGE_NOAUTOCLOSE
; 完成页：只保留「运行 Nuphus」。
; 桌面快捷方式由安装钩子默认创建，不再在完成页让用户重复勾选（2026-10-31 简化）。
; Show run app after installation.
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_FUNCTION RunMainBinary
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_FINISH

Function RunMainBinary
  nsis_tauri_utils::RunAsUser "$INSTDIR\${MAINBINARYNAME}.exe" ""
FunctionEnd

; Uninstaller Pages
; 1. Confirm uninstall page
Var DeleteAppDataCheckbox
Var DeleteAppDataCheckboxState
!define /ifndef WS_EX_LAYOUTRTL         0x00400000
!define MUI_PAGE_CUSTOMFUNCTION_SHOW un.ConfirmShow
Function un.ConfirmShow ; Add add a `Delete app data` check box
  ; $1 inner dialog HWND
  ; $2 window DPI
  ; $3 style
  ; $4 x
  ; $5 y
  ; $6 width
  ; $7 height
  FindWindow $1 "#32770" "" $HWNDPARENT ; Find inner dialog
  System::Call "user32::GetDpiForWindow(p r1) i .r2"
  ${If} $(^RTL) = 1
    StrCpy $3 "${__NSD_CheckBox_EXSTYLE} | ${WS_EX_LAYOUTRTL}"
    IntOp $4 50 * $2
  ${Else}
    StrCpy $3 "${__NSD_CheckBox_EXSTYLE}"
    IntOp $4 0 * $2
  ${EndIf}
  IntOp $5 100 * $2
  IntOp $6 400 * $2
  IntOp $7 25 * $2
  IntOp $4 $4 / 96
  IntOp $5 $5 / 96
  IntOp $6 $6 / 96
  IntOp $7 $7 / 96
  System::Call 'user32::CreateWindowEx(i r3, w "${__NSD_CheckBox_CLASS}", w "$(deleteAppData)", i ${__NSD_CheckBox_STYLE}, i r4, i r5, i r6, i r7, p r1, i0, i0, i0) i .s'
  Pop $DeleteAppDataCheckbox
  SendMessage $HWNDPARENT ${WM_GETFONT} 0 0 $1
  SendMessage $DeleteAppDataCheckbox ${WM_SETFONT} $1 1
FunctionEnd
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE un.ConfirmLeave
Function un.ConfirmLeave
  SendMessage $DeleteAppDataCheckbox ${BM_GETCHECK} 0 0 $DeleteAppDataCheckboxState
FunctionEnd
!define MUI_PAGE_CUSTOMFUNCTION_PRE un.SkipIfPassive
!insertmacro MUI_UNPAGE_CONFIRM

; 2. Uninstalling Page
!insertmacro MUI_UNPAGE_INSTFILES

;Languages
; PageReinstall reads the misspelled `choowHowToInstall` as a fallback, and makensis warns
; about a language string that is read but never set, so default it to an empty string for
; every language. This runs before the language files are included, so a custom language
; file that only defines the old spelling overrides the default and the fallback shows its
; text. `0` as the language id means the language inserted right above.
; TODO: remove the `choowHowToInstall` fallback in a future major version
{{#each languages}}
!insertmacro MUI_LANGUAGE "{{this}}"
LangString choowHowToInstall 0 ""
{{/each}}
!insertmacro MUI_RESERVEFILE_LANGDLL
{{#each language_files}}
  !include "{{this}}"
{{/each}}

; ═══════════════════════════════════════════════════════════════════════════
; Nuphus 自定义字符串(依赖检测页 + 失败原因人话化,2026-10-30)
;
; 用 !ifdef LANG_* 编译期守卫:仅在对应语言已启用
; (bundle.windows.nsis.languages)时才注册 —— 调整语言列表不会触发
; "未知 define ${LANG_*}" 编译错误。禁止与上游语言文件(上方 include)
; 已定义的名字(如 webview2Downloading)重复。
; ═══════════════════════════════════════════════════════════════════════════

!ifdef LANG_ENGLISH
LangString depCheckHeader ${LANG_ENGLISH} "Checking required runtime components"
LangString depCheckSub ${LANG_ENGLISH} "Missing system components are downloaded and installed automatically (from Microsoft's official site)"
LangString depCheckLead ${LANG_ENGLISH} "The following system components are checked before installation:"
LangString depWebview2Ok ${LANG_ENGLISH} "WebView2 Runtime - already installed"
LangString depWebview2Missing ${LANG_ENGLISH} "WebView2 Runtime - NOT detected. It will be downloaded (~2 MB) and installed automatically during setup."
LangString depWebview2StatusOk ${LANG_ENGLISH} "OK"
LangString depWebview2StatusMiss ${LANG_ENGLISH} "!"
LangString depVcOk ${LANG_ENGLISH} "Visual C++ Runtime - already installed"
LangString depVcMissing ${LANG_ENGLISH} "Visual C++ Runtime - NOT detected. It will be downloaded (~25 MB) and installed automatically during setup."
LangString depVcStatusOk ${LANG_ENGLISH} "OK"
LangString depVcStatusMiss ${LANG_ENGLISH} "!"
LangString depNote ${LANG_ENGLISH} "All components come directly from Microsoft's official servers. If a download fails, you can retry or abort - the installer will explain the reason, and the official manual download link is always shown."
LangString depFailureTitle ${LANG_ENGLISH} " download failed."
LangString depReasonBody ${LANG_ENGLISH} " Reason: "
LangString depReasonCancel ${LANG_ENGLISH} "The transfer was cancelled (connection dropped or cancelled in the progress window)."
LangString depReasonConnect ${LANG_ENGLISH} "Could not connect to the server (firewall / proxy / offline)."
LangString depReasonResolve ${LANG_ENGLISH} "Could not resolve the server address (DNS name or network unreachable)."
LangString depReasonTimeout ${LANG_ENGLISH} "The connection or download timed out."
LangString depReasonWrite ${LANG_ENGLISH} "Could not write the temporary file (disk space / antivirus)."
LangString depReasonGeneric ${LANG_ENGLISH} "Network restricted or error."
LangString depRawPrefix ${LANG_ENGLISH} "Original error: "
LangString depRetryGuide ${LANG_ENGLISH} "Press Retry to try again, or Cancel to abort the installation."
LangString depAbortGuide ${LANG_ENGLISH} "Setup has been aborted. Nuphus cannot run without this component - install it manually, then run this installer again."
LangString depManualWebview2 ${LANG_ENGLISH} "Manual download (official): https://go.microsoft.com/fwlink/p/?LinkId=2124703"
LangString depManualVc ${LANG_ENGLISH} "Manual download (official): https://aka.ms/vs/17/release/vc_redist.x64.exe"
LangString depTopicWebview2 ${LANG_ENGLISH} "WebView2 Runtime"
LangString depTopicVc ${LANG_ENGLISH} "Visual C++ Runtime"
LangString webview2ExecFailLead ${LANG_ENGLISH} "WebView2 bootstrapper install failed (exit code "
LangString webview2ExecFailTail ${LANG_ENGLISH} "). Common causes: antivirus blocking, network restriction, system policy."
LangString vcDownloading ${LANG_ENGLISH} "Downloading the Visual C++ runtime (~25 MB, from Microsoft) ..."
LangString vcInstalling ${LANG_ENGLISH} "Installing the Visual C++ runtime ..."
LangString vcInstallSuccess ${LANG_ENGLISH} "Visual C++ runtime installed successfully"
LangString vcAlreadyNewer ${LANG_ENGLISH} "A newer version of the Visual C++ runtime is already installed; nothing to do"
LangString vcRebootNeeded ${LANG_ENGLISH} "Visual C++ runtime installed successfully (a restart may be required before it fully applies)"
LangString vcElevating ${LANG_ENGLISH} "Administrator permission is required for the Visual C++ runtime. Please confirm the Windows UAC prompt ..."
LangString vcExecFailLead ${LANG_ENGLISH} "Visual C++ runtime install failed (exit code "
LangString vcExecFailTail ${LANG_ENGLISH} ")."
LangString vcReasonUacDenied ${LANG_ENGLISH} "Administrator permission was not granted (the UAC prompt was denied or unavailable) - the runtime was NOT installed. Retry and accept the prompt, or install it manually."
LangString vcReasonAfterElevate ${LANG_ENGLISH} "Installation still failed after elevation was granted. It may be blocked by antivirus software or system policy."
LangString vcBlockedReason ${LANG_ENGLISH} "Setup is blocked by system policy, or this OS is not supported by the runtime installer."
!endif

!ifdef LANG_SIMPCHINESE
LangString depCheckHeader ${LANG_SIMPCHINESE} "正在检查必要系统组件"
LangString depCheckSub ${LANG_SIMPCHINESE} "缺失的系统组件将自动下载安装(来自微软官方网站)"
LangString depCheckLead ${LANG_SIMPCHINESE} "安装前将检测以下系统组件:"
LangString depWebview2Ok ${LANG_SIMPCHINESE} "WebView2 运行库 —— 已安装"
LangString depWebview2Missing ${LANG_SIMPCHINESE} "WebView2 运行库 —— 未检测到,安装时将自动下载并安装(约 2 MB)"
LangString depWebview2StatusOk ${LANG_SIMPCHINESE} "√"
LangString depWebview2StatusMiss ${LANG_SIMPCHINESE} "×"
LangString depVcOk ${LANG_SIMPCHINESE} "VC++ 通用运行库 —— 已安装(OCR/语音引擎依赖)"
LangString depVcMissing ${LANG_SIMPCHINESE} "VC++ 通用运行库 —— 未检测到,安装时将自动下载并安装(约 25 MB)"
LangString depVcStatusOk ${LANG_SIMPCHINESE} "√"
LangString depVcStatusMiss ${LANG_SIMPCHINESE} "×"
LangString depNote ${LANG_SIMPCHINESE} "以上组件均直接从微软官方服务器获取,不会让所有用户背上离线包的大体积。若下载失败,可以选择重试或中止,安装器会说明具体原因,并始终提供官方手动下载地址。"
LangString depFailureTitle ${LANG_SIMPCHINESE} "下载失败。"
LangString depReasonBody ${LANG_SIMPCHINESE} "原因:"
LangString depReasonCancel ${LANG_SIMPCHINESE} "传输被取消(连接中断,或在下载进度窗中取消)。"
LangString depReasonConnect ${LANG_SIMPCHINESE} "无法连接服务器(防火墙 / 代理 / 断网)。"
LangString depReasonResolve ${LANG_SIMPCHINESE} "无法解析服务器地址(DNS 或网络不可达)。"
LangString depReasonTimeout ${LANG_SIMPCHINESE} "连接或下载超时。"
LangString depReasonWrite ${LANG_SIMPCHINESE} "无法写入临时文件(磁盘空间 / 杀毒软件)。"
LangString depReasonGeneric ${LANG_SIMPCHINESE} "网络受限或出错。"
LangString depRawPrefix ${LANG_SIMPCHINESE} "原始错误:"
LangString depRetryGuide ${LANG_SIMPCHINESE} "点「重试」再试一次,或「取消」中止安装。"
LangString depAbortGuide ${LANG_SIMPCHINESE} "安装已中止:缺少该组件 Nuphus 无法正常运行。请手动安装后重新运行本安装包。"
LangString depManualWebview2 ${LANG_SIMPCHINESE} "手动下载(官方):https://go.microsoft.com/fwlink/p/?LinkId=2124703"
LangString depManualVc ${LANG_SIMPCHINESE} "手动下载(官方):https://aka.ms/vs/17/release/vc_redist.x64.exe"
LangString depTopicWebview2 ${LANG_SIMPCHINESE} "WebView2 运行库"
LangString depTopicVc ${LANG_SIMPCHINESE} "VC++ 运行库"
LangString webview2ExecFailLead ${LANG_SIMPCHINESE} "WebView2 引导程序安装失败(退出码 "
LangString webview2ExecFailTail ${LANG_SIMPCHINESE} ")。常见原因:安全软件拦截、网络受限、系统策略限制。"
LangString vcDownloading ${LANG_SIMPCHINESE} "正在下载 VC++ 运行库(约 25 MB,来自微软官网)…"
LangString vcInstalling ${LANG_SIMPCHINESE} "正在安装 VC++ 运行库…"
LangString vcInstallSuccess ${LANG_SIMPCHINESE} "VC++ 运行库安装完成"
LangString vcAlreadyNewer ${LANG_SIMPCHINESE} "系统已装有更新版本的 VC++ 运行库,无需安装"
LangString vcRebootNeeded ${LANG_SIMPCHINESE} "VC++ 运行库安装成功(部分组件可能需要重启后生效)"
LangString vcElevating ${LANG_SIMPCHINESE} "VC++ 运行库需要管理员权限安装 —— 请在系统弹出的授权框(UAC)中确认…"
LangString vcExecFailLead ${LANG_SIMPCHINESE} "VC++ 运行库安装失败(退出码 "
LangString vcExecFailTail ${LANG_SIMPCHINESE} ")。"
LangString vcReasonUacDenied ${LANG_SIMPCHINESE} "未获得管理员授权(UAC 被拒绝或不可用)—— VC++ 运行库未安装。重试并在弹窗中确认,或联系管理员。"
LangString vcReasonAfterElevate ${LANG_SIMPCHINESE} "已获授权但安装仍失败。可能是安全软件拦截或系统策略限制。"
LangString vcBlockedReason ${LANG_SIMPCHINESE} "VC++ 运行库被系统策略阻止安装,或当前操作系统不受该安装器支持。"
!endif

Function .onInit
  ${GetOptions} $CMDLINE "/P" $PassiveMode
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/NS" $NoShortcutMode
  ${IfNot} ${Errors}
    StrCpy $NoShortcutMode 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}

  !if "${DISPLAYLANGUAGESELECTOR}" == "true"
    !insertmacro MUI_LANGDLL_DISPLAY
  !endif

  !insertmacro SetContext

  ${If} $INSTDIR == "${PLACEHOLDER_INSTALL_DIR}"
    ; Set default install location
    !if "${INSTALLMODE}" == "perMachine"
      ${If} ${RunningX64}
        !if "${ARCH}" == "x64"
          StrCpy $INSTDIR "$PROGRAMFILES64\${PRODUCTNAME}"
        !else if "${ARCH}" == "arm64"
          StrCpy $INSTDIR "$PROGRAMFILES64\${PRODUCTNAME}"
        !else
          StrCpy $INSTDIR "$PROGRAMFILES\${PRODUCTNAME}"
        !endif
      ${Else}
        StrCpy $INSTDIR "$PROGRAMFILES\${PRODUCTNAME}"
      ${EndIf}
    !else if "${INSTALLMODE}" == "currentUser"
      StrCpy $INSTDIR "$LOCALAPPDATA\${PRODUCTNAME}"
    !endif

    Call RestorePreviousInstallLocation
  ${EndIf}


  !if "${INSTALLMODE}" == "both"
    !insertmacro MULTIUSER_INIT
  !endif

  ; Abort downgrades if they are disabled（2026-10-31：维护页移除后，
  ; 交互安装不再有「卸载后再装」分支，降级会静默覆盖新版本 → 全模式统一拦截）
  !if "${ALLOWDOWNGRADES}" == "false"
    ; Compare with the same installation the reinstall page would: a WiX one first
    Call FindWixInstall
    ${If} $R6 != ""
      ReadRegStr $R0 HKLM "$R6" "DisplayVersion"
    ${Else}
      ReadRegStr $R0 SHCTX "${UNINSTKEY}" "DisplayVersion"
    ${EndIf}
    nsis_tauri_utils::SemverCompare "${VERSION}" $R0
    Pop $R0
    ; If downgrading
    ${If} $R0 = -1
      ${If} ${Silent}
        System::Call 'kernel32::AttachConsole(i -1)i.r0'
        ${If} $0 <> 0
          System::Call 'kernel32::GetStdHandle(i -11)i.r0'
          System::call 'kernel32::SetConsoleTextAttribute(i r0, i 0x0004)' ; set red color
          FileWrite $0 "$(silentDowngrades)"
        ${EndIf}
      ${Else}
        MessageBox MB_ICONSTOP "$(silentDowngrades)"
      ${EndIf}
      Abort
    ${EndIf}
  !endif
FunctionEnd

Section WebView2
  ; Check if Webview2 is already installed and skip this section
  ${If} ${RunningX64}
    ReadRegStr $4 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${Else}
    ReadRegStr $4 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${EndIf}
  ${If} $4 == ""
    ReadRegStr $4 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${EndIf}

  ${If} $4 == ""
    ; Webview2 installation
    ;
    ; Skip if updating
    ${If} $UpdateMode <> 1
      !if "${INSTALLWEBVIEW2MODE}" == "downloadBootstrapper"
        ; ── Nuphus 改造:下载失败不再硬中止 ─────────────────────────
        ; 原版:NSISdl 失败 → Abort(无原因,用户一头雾水)。
        ; 现在:返回码归因成人话 + [重试/取消] 自选;
        ;       取消时明示手动获取地址与后果(缺 WebView2 应用无法运行)。
        webview2_download_retry:
          Delete "$TEMP\MicrosoftEdgeWebview2Setup.exe"
          DetailPrint "$(webview2Downloading)"
          NSISdl::download /TIMEOUT=45000 "https://go.microsoft.com/fwlink/p/?LinkId=2124703" "$TEMP\MicrosoftEdgeWebview2Setup.exe"
          Pop $0
          StrCmp $0 "success" webview2_download_ok 0
          StrCpy $R7 "$(depTopicWebview2)"
          Call DescribeNsisdlDownloadFailure   ; → $1 = 完整提示
          MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$1" /SD IDCANCEL
          Pop $2
          StrCmp $2 "2" webview2_download_leftdown webview2_download_retry ; IDCANCEL=2
        webview2_download_ok:
          DetailPrint "$(webview2DownloadSuccess)"
          StrCpy $6 "$TEMP\MicrosoftEdgeWebview2Setup.exe"
          Goto install_webview2
        webview2_download_leftdown:
          DetailPrint "$(depManualWebview2)"
          Abort "$(depManualWebview2)$\n$\n$(depAbortGuide)"
      !endif

      !if "${INSTALLWEBVIEW2MODE}" == "embedBootstrapper"
        Delete "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        File "/oname=$TEMP\MicrosoftEdgeWebview2Setup.exe" "${WEBVIEW2BOOTSTRAPPERPATH}"
        DetailPrint "$(installingWebview2)"
        StrCpy $6 "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        Goto install_webview2
      !endif

      !if "${INSTALLWEBVIEW2MODE}" == "offlineInstaller"
        Delete "$TEMP\MicrosoftEdgeWebView2RuntimeInstaller.exe"
        File "/oname=$TEMP\MicrosoftEdgeWebView2RuntimeInstaller.exe" "${WEBVIEW2INSTALLERPATH}"
        DetailPrint "$(installingWebview2)"
        StrCpy $6 "$TEMP\MicrosoftEdgeWebView2RuntimeInstaller.exe"
        Goto install_webview2
      !endif

      Goto webview2_done

      install_webview2:
        DetailPrint "$(installingWebview2)"
        ; $6 holds the path to the webview2 installer
        ; ── Nuphus 改造 ──
        ; 1) 引导器加 /silent:全自动无弹窗(退出码可判定;原版不带 /silent
        ;    会弹 WebView2 自家安装窗,与我们"安装器一把手"的口径冲突);
        ; 2) 退出码非 0 时先复核注册表 pv —— 引导器存在"提前退出"的已知
        ;    行为(WebView2Feedback#1349),pv 已写入即视为成功;
        ; 3) 失败给出退出码画像 + 可重试(原版硬 Abort)。
        webview2_exec_retry:
        ExecWait '$6 /silent ${WEBVIEW2INSTALLERARGS} /install' $1
        ${If} $1 = 0
          DetailPrint "$(webview2InstallSuccess)"
        ${Else}
          Call ReadWebview2Pv
          ${If} $4 != ""
            DetailPrint "$(webview2InstallSuccess)"   ; 引导器提前退出,但运行库已经装上
          ${Else}
            StrCpy $2 "$(webview2ExecFailLead)$1$(webview2ExecFailTail)"
            MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$2" /SD IDCANCEL
            Pop $2
            StrCmp $2 "2" webview2_exec_leftdown webview2_exec_retry ; IDCANCEL=2
          ${EndIf}
        ${EndIf}
        Goto webview2_done
        webview2_exec_leftdown:
          DetailPrint "$(depManualWebview2)"
          Abort "$(depManualWebview2)$\n$\n$(depAbortGuide)"
      webview2_done:
    ${EndIf}
  ${Else}
    !if "${MINIMUMWEBVIEW2VERSION}" != ""
      ${VersionCompare} "${MINIMUMWEBVIEW2VERSION}" "$4" $R0
      ${If} $R0 = 1
        update_webview:
          DetailPrint "$(installingWebview2)"
          ${If} ${RunningX64}
            ReadRegStr $R1 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate" "path"
          ${Else}
            ReadRegStr $R1 HKLM "SOFTWARE\Microsoft\EdgeUpdate" "path"
          ${EndIf}
          ${If} $R1 == ""
            ReadRegStr $R1 HKCU "SOFTWARE\Microsoft\EdgeUpdate" "path"
          ${EndIf}
          ${If} $R1 != ""
            ; Chromium updater docs: https://source.chromium.org/chromium/chromium/src/+/main:docs/updater/user_manual.md
            ; Modified from "HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Microsoft EdgeWebView\ModifyPath"
            ExecWait `"$R1" /install appguid=${WEBVIEW2APPGUID}&needsadmin=true` $1
            ${If} $1 = 0
              DetailPrint "$(webview2InstallSuccess)"
            ${Else}
              MessageBox MB_ICONEXCLAMATION|MB_ABORTRETRYIGNORE "$(webview2InstallError)" IDIGNORE ignore IDRETRY update_webview
              Quit
              ignore:
            ${EndIf}
          ${EndIf}
      ${EndIf}
    !endif
  ${EndIf}
SectionEnd

; ═══════════════════════════════════════════════════════════════════════════
; Section -vcruntime(Nuphus 自定义,2026-10-30)
;
; 为什么需要:本体 nuphus.exe 静态链接 CRT 不依赖 VC++ 运行库,但随包的
; 原生引擎 onnxruntime.dll(OCR)/ sherpa-onnx-c-api.dll(语音)在导入
; 表中显式依赖以下三件(VCRUNTIME140.dll / VCRUNTIME140_1.dll /
; MSVCP140.dll)—— 缺 VC++ 运行库的机器上这些功能会静默失败。
;
; 策略(与 WebView2 相同):检测 → 缺失才下载安装(vc_redist ~25MB,
; 微软官方永久链接),不让所有用户背上体积;安装失败走到哪一步失败,
; 原因都要说人话,并且允许重试或中止。
;
; 退出码映射(微软 VC++ Redistributable 文档):
;   0     安装成功
;   1638  已装有更高版本(直接放行)
;   3010  安装成功,需重启生效
;   5100  被系统策略阻止
;   其他  失败—— 改走 UAC 授权通道重试(非管理中心安装时最常见原因是权限不足)
Section "-vcruntime"
  Call DetectVcRuntime
  ${If} $R9 = 0
    Goto vc_done
  ${EndIf}
  ; 更新模式(应用内自动升级)不触碰系统运行库:首装已完成安装
  ${If} $UpdateMode = 1
    Goto vc_done
  ${EndIf}

  ; ── 阶段 1:下载 vc_redist(微软官方永久链接)──
  vc_download_retry:
    Delete "$TEMP\vc_redist.x64.exe"
    DetailPrint "$(vcDownloading)"
    NSISdl::download /TIMEOUT=45000 "https://aka.ms/vs/17/release/vc_redist.x64.exe" "$TEMP\vc_redist.x64.exe"
    Pop $0
    StrCmp $0 "success" vc_download_ok 0
    StrCpy $R7 "$(depTopicVc)"
    Call DescribeNsisdlDownloadFailure   ; → $1 = 含原因的人话提示
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$1" /SD IDCANCEL
    Pop $2
    StrCmp $2 "2" vc_leftdown vc_download_retry ; IDCANCEL=2

  ; ── 阶段 2:静默安装(vc_redist /install /quiet /norestart)──
  vc_download_ok:
  vc_exec_retry:
    DetailPrint "$(vcInstalling)"
    ExecWait '"$TEMP\vc_redist.x64.exe" /install /quiet /norestart' $1
    ; 退出码画像
    StrCmp $1 "0" vc_ok 0
    StrCmp $1 "1638" vc_newer 0
    StrCmp $1 "3010" vc_reboot 0
    StrCmp $1 "5100" vc_blocked 0
    ; ── 阶段 3:权限兜底 ──
    ; 本安装器 RequestExecutionLevel=user(装在用户目录),
    ; vc_redist 是机器级系统组件,需要管理员授权。
    ; ExecWait(CreateProcess)触发不了 UAC,用 ExecShellWait「runas」
    ; 动词请求系统授权弹窗,完成后由注册表实测结果兜底判定。
    DetailPrint "$(vcElevating)"
    ClearErrors
    ExecShellWait "runas" "$TEMP\vc_redist.x64.exe" "/install /quiet /norestart" SW_SHOW
    ${If} ${Errors}
      ; UAC 被拒绝/不可得 —— 原因明示
      StrCpy $2 "$(vcExecFailLead)$1$(vcExecFailTail)$\n$\n$(vcReasonUacDenied)"
    ${Else}
      ; 引擎(Burn)可能异步收尾:15 秒宽限内两次复检注册表
      Sleep 1500
      Call DetectVcRuntime
      ${If} $R9 = 0
        Goto vc_ok
      ${EndIf}
      Sleep 4000
      Call DetectVcRuntime
      ${If} $R9 = 0
        Goto vc_ok
      ${EndIf}
      StrCpy $2 "$(vcExecFailLead)$1$(vcExecFailTail)$\n$\n$(vcReasonAfterElevate)"
    ${EndIf}
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$2" /SD IDCANCEL
    Pop $2
    StrCmp $2 "2" vc_leftdown vc_exec_retry ; IDCANCEL=2

  ; ── 成功 / 终态 ──
  vc_ok:
    DetailPrint "$(vcInstallSuccess)"
    Goto vc_done
  vc_newer:
    DetailPrint "$(vcAlreadyNewer)"
    Goto vc_done
  vc_reboot:
    DetailPrint "$(vcRebootNeeded)"
    Goto vc_done
  vc_blocked:
    DetailPrint "$(depManualVc)"
    Abort "$(vcBlockedReason)$\n$\n$(depManualVc)$\n$\n$(depAbortGuide)"
  vc_leftdown:
    DetailPrint "$(depManualVc)"
    Abort "$(depManualVc)$\n$\n$(depAbortGuide)"
  vc_done:
    Delete "$TEMP\vc_redist.x64.exe"
SectionEnd

Section Install
  SetOutPath $INSTDIR

  !ifmacrodef NSIS_HOOK_PREINSTALL
    !insertmacro NSIS_HOOK_PREINSTALL
  !endif

  !insertmacro CheckIfAppIsRunning "$INSTDIR\${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

  ; Copy main executable
  File "${MAINBINARYSRCPATH}"

  ; Copy resources
  {{#each resources_dirs}}
    CreateDirectory "$INSTDIR\\{{this}}"
  {{/each}}
  {{#each resources}}
    File /a "/oname={{this.[1]}}" "{{no-escape @key}}"
  {{/each}}

  ; Copy external binaries
  {{#each binaries}}
    File /a "/oname={{this}}" "{{no-escape @key}}"
  {{/each}}

  ; Create file associations
  {{#each file_associations as |association| ~}}
    {{#each association.ext as |ext| ~}}
       !insertmacro APP_ASSOCIATE "{{ext}}" "{{or association.name ext}}" "{{association-description association.description ext}}" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0" "Open with ${PRODUCTNAME}" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
    {{/each}}
  {{/each}}

  ; Register deep links
  {{#each deep_link_protocols as |protocol| ~}}
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}" "URL Protocol" ""
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}" "" "URL:${BUNDLEID} protocol"
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
  {{/each}}

  ; Create uninstaller
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Save $INSTDIR in registry for future installations
  WriteRegStr SHCTX "${MANUPRODUCTKEY}" "" $INSTDIR

  !if "${INSTALLMODE}" == "both"
    ; Save install mode to be selected by default for the next installation such as updating
    ; or when uninstalling
    WriteRegStr SHCTX "${UNINSTKEY}" $MultiUser.InstallMode 1
  !endif

  ; Remove old main binary if it doesn't match new main binary name
  ReadRegStr $OldMainBinaryName SHCTX "${UNINSTKEY}" "MainBinaryName"
  ${If} $OldMainBinaryName != ""
  ${AndIf} $OldMainBinaryName != "${MAINBINARYNAME}.exe"
    Delete "$INSTDIR\$OldMainBinaryName"
  ${EndIf}

  ; Save current MAINBINARYNAME for future updates
  WriteRegStr SHCTX "${UNINSTKEY}" "MainBinaryName" "${MAINBINARYNAME}.exe"

  ; Registry information for add/remove programs
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayIcon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr SHCTX "${UNINSTKEY}" "Publisher" "${MANUFACTURER}"
  WriteRegStr SHCTX "${UNINSTKEY}" "InstallLocation" "$\"$INSTDIR$\""
  WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoModify" "1"
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoRepair" "1"

  ${GetSize} "$INSTDIR" "/M=uninstall.exe /S=0K /G=0" $0 $1 $2
  IntOp $0 $0 + ${ESTIMATEDSIZE}
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD SHCTX "${UNINSTKEY}" "EstimatedSize" "$0"

  !if "${HOMEPAGE}" != ""
    WriteRegStr SHCTX "${UNINSTKEY}" "URLInfoAbout" "${HOMEPAGE}"
    WriteRegStr SHCTX "${UNINSTKEY}" "URLUpdateInfo" "${HOMEPAGE}"
    WriteRegStr SHCTX "${UNINSTKEY}" "HelpLink" "${HOMEPAGE}"
  !endif

  ; Create start menu shortcut
  !insertmacro MUI_STARTMENU_WRITE_BEGIN Application
    Call CreateOrUpdateStartMenuShortcut
  !insertmacro MUI_STARTMENU_WRITE_END

  ; Create desktop shortcut for silent and passive installers
  ; because finish page will be skipped
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    Call CreateOrUpdateDesktopShortcut
  ${EndIf}

  !ifmacrodef NSIS_HOOK_POSTINSTALL
    !insertmacro NSIS_HOOK_POSTINSTALL
  !endif

  ; Auto close this page for passive mode
  ${If} $PassiveMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd

Function .onInstSuccess
  ; Check for `/R` flag only in silent and passive installers because
  ; GUI installer has a toggle for the user to (re)start the app
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    ${GetOptions} $CMDLINE "/R" $R0
    ${IfNot} ${Errors}
      ${GetOptions} $CMDLINE "/ARGS" $R0
      nsis_tauri_utils::RunAsUser "$INSTDIR\${MAINBINARYNAME}.exe" "$R0"
    ${EndIf}
  ${EndIf}
FunctionEnd

Function un.onInit
  !insertmacro SetContext

  !if "${INSTALLMODE}" == "both"
    !insertmacro MULTIUSER_UNINIT
  !endif

  !insertmacro MUI_UNGETLANGUAGE

  ${GetOptions} $CMDLINE "/P" $PassiveMode
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
FunctionEnd

Section Uninstall

  !ifmacrodef NSIS_HOOK_PREUNINSTALL
    !insertmacro NSIS_HOOK_PREUNINSTALL
  !endif

  !insertmacro CheckIfAppIsRunning "$INSTDIR\${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

  ; Delete the app directory and its content from disk
  ; Copy main executable
  Delete "$INSTDIR\${MAINBINARYNAME}.exe"

  ; Delete resources
  {{#each resources}}
    Delete "$INSTDIR\\{{this.[1]}}"
  {{/each}}

  ; Delete external binaries
  {{#each binaries}}
    Delete "$INSTDIR\\{{this}}"
  {{/each}}

  ; Delete app associations
  {{#each file_associations as |association| ~}}
    {{#each association.ext as |ext| ~}}
      !insertmacro APP_UNASSOCIATE "{{ext}}" "{{or association.name ext}}"
    {{/each}}
  {{/each}}

  ; Delete deep links
  {{#each deep_link_protocols as |protocol| ~}}
    ReadRegStr $R7 SHCTX "Software\Classes\\{{protocol}}\shell\open\command" ""
    ${If} $R7 == "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
      DeleteRegKey SHCTX "Software\Classes\\{{protocol}}"
    ${EndIf}
  {{/each}}


  ; Delete uninstaller
  Delete "$INSTDIR\uninstall.exe"

  {{#each resources_ancestors}}
  RMDir /REBOOTOK "$INSTDIR\\{{this}}"
  {{/each}}
  RMDir "$INSTDIR"

  ; Remove shortcuts if not updating
  ${If} $UpdateMode <> 1
    !insertmacro DeleteAppUserModelId

    ; Remove start menu shortcut
    !insertmacro MUI_STARTMENU_GETFOLDER Application $AppStartMenuFolder
    !insertmacro IsShortcutTarget "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      !insertmacro UnpinShortcut "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
      Delete "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
      RMDir "$SMPROGRAMS\$AppStartMenuFolder"
    ${EndIf}
    !insertmacro IsShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      !insertmacro UnpinShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk"
      Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    ${EndIf}

    ; Remove desktop shortcuts
    !insertmacro IsShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      !insertmacro UnpinShortcut "$DESKTOP\${PRODUCTNAME}.lnk"
      Delete "$DESKTOP\${PRODUCTNAME}.lnk"
    ${EndIf}
  ${EndIf}

  ; Remove registry information for add/remove programs
  !if "${INSTALLMODE}" == "both"
    DeleteRegKey SHCTX "${UNINSTKEY}"
  !else if "${INSTALLMODE}" == "perMachine"
    DeleteRegKey HKLM "${UNINSTKEY}"
  !else
    DeleteRegKey HKCU "${UNINSTKEY}"
  !endif

  ; Removes the Autostart entry for ${PRODUCTNAME} from the HKCU Run key if it exists.
  ; This ensures the program does not launch automatically after uninstallation if it exists.
  ; If it doesn't exist, it does nothing.
  ; We do this when not updating (to preserve the registry value on updates)
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"
  ${EndIf}

  ; Delete app data if the checkbox is selected
  ; and if not updating
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    ; Clear the install location $INSTDIR from registry
    DeleteRegKey SHCTX "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty SHCTX "${MANUKEY}"

    ; Clear the install language from registry
    DeleteRegValue HKCU "${MANUPRODUCTKEY}" "Installer Language"
    DeleteRegKey /ifempty HKCU "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty HKCU "${MANUKEY}"

    SetShellVarContext current
    RmDir /r "$APPDATA\${BUNDLEID}"
    RmDir /r "$LOCALAPPDATA\${BUNDLEID}"
  ${EndIf}

  !ifmacrodef NSIS_HOOK_POSTUNINSTALL
    !insertmacro NSIS_HOOK_POSTUNINSTALL
  !endif

  ; Auto close if passive mode or updating
  ${If} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd

Function RestorePreviousInstallLocation
  ReadRegStr $4 SHCTX "${MANUPRODUCTKEY}" ""
  StrCmp $4 "" +2 0
    StrCpy $INSTDIR $4
FunctionEnd

Function Skip
  Abort
FunctionEnd

Function SkipIfPassive
  ${IfThen} $PassiveMode = 1  ${|} Abort ${|}
FunctionEnd
Function un.SkipIfPassive
  ${IfThen} $PassiveMode = 1  ${|} Abort ${|}
FunctionEnd

Function CreateOrUpdateStartMenuShortcut
  ; We used to use product name as MAINBINARYNAME
  ; migrate old shortcuts to target the new MAINBINARYNAME
  StrCpy $R0 0

  !insertmacro IsShortcutTarget "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\$OldMainBinaryName"
  Pop $0
  ${If} $0 = 1
    !insertmacro SetShortcutTarget "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    StrCpy $R0 1
  ${EndIf}

  !insertmacro IsShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\$OldMainBinaryName"
  Pop $0
  ${If} $0 = 1
    !insertmacro SetShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    StrCpy $R0 1
  ${EndIf}

  ${If} $R0 = 1
    Return
  ${EndIf}

  ; Skip creating shortcut if in update mode or no shortcut mode
  ; but always create if migrating from wix
  ${If} $WixMode = 0
    ${If} $UpdateMode = 1
    ${OrIf} $NoShortcutMode = 1
      Return
    ${EndIf}
  ${EndIf}

  !if "${STARTMENUFOLDER}" != ""
    CreateDirectory "$SMPROGRAMS\$AppStartMenuFolder"
    CreateShortcut "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
  !else
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  !endif
FunctionEnd

Function CreateOrUpdateDesktopShortcut
  ; We used to use product name as MAINBINARYNAME
  ; migrate old shortcuts to target the new MAINBINARYNAME
  !insertmacro IsShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\$OldMainBinaryName"
  Pop $0
  ${If} $0 = 1
    !insertmacro SetShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Return
  ${EndIf}

  ; Skip creating shortcut if in update mode or no shortcut mode
  ; but always create if migrating from wix
  ${If} $WixMode = 0
    ${If} $UpdateMode = 1
    ${OrIf} $NoShortcutMode = 1
      Return
    ${EndIf}
  ${EndIf}

  CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
FunctionEnd
