; installer-hooks.nsh — Nuphus 安装器钩子
;
; ── 1. 桌面快捷方式默认创建 ─────────────────────────────────────────────
; 完成页上的「创建桌面快捷方式」是可选动作,用户不勾就没有图标——
; 「装完之后不知道从哪启动」的体验缺陷(2026-10-30 大王验收安装流程)。
; 安装完成后**默认创建**,同时保留完成页选项的幂等兼容(同名同目标)。
; 与模板删除逻辑完全兼容:卸载时模板按 IsShortcutTarget($DESKTOP\Nuphus.lnk,
; 目标 $INSTDIR\nuphus.exe)清理,名字与目标同模板语义,不产生孤儿快捷方式。
;   - passive(应用内更新)不重复:模板 Skipped 完成页时已**无条件**创建桌面
;     快捷方式(见 installer.nsi Section Install 结尾 ${If} $PassiveMode),
;     这里跳过避免重复写同一路径;
;   - /NS(no-shortcuts)命令行开关:尊重用户「不要快捷方式」的意愿。
; ── 2. 图标缓存刷新 ────────────────────────────────────────────────────
; 安装/升级完成后广播一次 SHCNE_ASSOCCHANGED,强制 Explorer 重建图标缓存。
;
; 为什么需要:Windows 的图标缓存以「文件路径 + 时间戳/大小」为键。安装包替换
; nuphus.exe 的瞬间若 Explorer 恰好去读图标,会读到不完整的 PE 资源并回退成
; 通用白纸图标,然后一直沿用。Tauri 的 NSIS 模板创建快捷方式后不做任何刷新,
; 是否重读完全依赖 Explorer 自身,这里补上确定性动作。
;
; 覆盖范围:首次安装、手动升级、Tauri updater 自动更新(同样走 NSIS)。
; 常量:SHCNE_ASSOCCHANGED = 0x08000000,SHCNF_IDLIST = 0x0000。
; 该 API 为用户级,不需要管理员权限。

!macro NSIS_HOOK_POSTINSTALL
  ; 1) 桌面快捷方式默认创建($PassiveMode/$NoShortcutMode 由主模板声明)
  ${If} $PassiveMode <> 1
  ${AndIf} $NoShortcutMode <> 1
    ; 等价于模板 CreateOrUpdateDesktopShortcut 的语义(不迁移旧快捷方式时):
    CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
  ${EndIf}

  ; 2) 广播图标缓存刷新(原行为,保持不变)
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0x0000, i 0, i 0)'
!macroend
