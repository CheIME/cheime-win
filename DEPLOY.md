# CheIME Windows 部署指南

## 前提

- Windows 10/11 x64
- PowerShell 7 (或 Windows PowerShell 5.1)
- 管理员权限（注册 TIP 需要写 `HKEY_CLASSES_ROOT`）

## 方法1：一键脚本部署

```powershell
cd D:\coding\cheime\cheime-win

# 构建 + 安装
.\scripts\build.ps1
.\scripts\install.ps1

# 启动引擎
%LOCALAPPDATA%\CheIME\bin\cheime-engine.exe --dict-dir %LOCALAPPDATA%\CheIME\data\dicts
```

## 方法2：手动部署

### 步骤1：构建

```powershell
cd D:\coding\cheime\cheime-win
cargo build --release
```

产物在 `target\release\`:

- `cheime-engine.exe` — 引擎进程
- `cheime_tip.dll` — TIP COM DLL  
- `cheime-installer.exe` — 安装工具

### 步骤2：复制文件

```powershell
$dst = "$env:LOCALAPPDATA\CheIME"
mkdir -Force "$dst\bin", "$dst\data\dicts", "$dst\config"

copy -Force target\release\cheime-engine.exe    "$dst\bin\"
copy -Force target\release\cheime_tip.dll       "$dst\bin\cheime-tip.dll"
copy -Force target\release\cheime-installer.exe "$dst\bin\"
copy -Force data\dicts\*                        "$dst\data\dicts\"
```

### 步骤3：注册 TIP DLL

**方案A — regsvr32（推荐）**：

```powershell
cd "$env:LOCALAPPDATA\CheIME\bin"
regsvr32.exe cheime-tip.dll
```

**方案B — 手动注册表**（如果 DllRegisterServer 未包含完整注册表逻辑）：

写以下键（存为 `cheime-reg.reg` 双击导入）：

```reg
Windows Registry Editor Version 5.00

[HKEY_CLASSES_ROOT\CLSID\{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}]
@="CheIME TIP"

[HKEY_CLASSES_ROOT\CLSID\{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}\InprocServer32]
@="C:\\Users\\<你的用户名>\\AppData\\Local\\CheIME\\bin\\cheime-tip.dll"
"ThreadingModel"="Apartment"

[HKEY_CLASSES_ROOT\CLSID\{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}\Implemented Categories\{34745C63-B2F0-4784-8B67-5E12C8701A31}]
@=""
```

注意：把路径中的 `<你的用户名>` 替换为实际的 Windows 用户名。

### 步骤4：注册 TSF Profile

```powershell
# 存为 tsf-profile.reg，双击导入
```

```reg
Windows Registry Editor Version 5.00

[HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\CTF\TIP\{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}\LanguageProfile\0x00000804\{D7E2A3B4-C5F6-7890-ABCD-EF1234567890}]
@="CheIME 中文输入法"
"Description"="CheIME Chinese Input Method"
"EnableCategory"=dword:00000001
```

### 步骤5：启动引擎

```powershell
%LOCALAPPDATA%\CheIME\bin\cheime-engine.exe --dict-dir %LOCALAPPDATA%\CheIME\data\dicts
```

### 步骤6：切换输入法

1. 打开 **设置 → 时间和语言 → 语言和区域**
2. 找到 **中文(简体)** → 选项 → 添加键盘
3. 应该能看到 **CheIME 中文输入法**
4. 选中后，`Win+Space` 即可切换

## 验证

```powershell
# 快速引擎逻辑验证（不注册 TIP 也能跑）
echo '{"KeyCommand":{"header":{"protocol_version":1,"client":1,"session":1,"epoch":1,"sequence":1,"revision":0,"deployment":1},"event":{"key":{"Character":"n"},"state":{"shift":false,"control":false,"alt":false}}}}' | cheime-engine.exe --stdin
```

## 卸载

```powershell
# 注销 TIP
regsvr32.exe /u cheime-tip.dll

# 或者手动删注册表
reg delete "HKCR\CLSID\{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}" /f
reg delete "HKLM\SOFTWARE\Microsoft\CTF\TIP\{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}" /f

# 删文件
rm -Recurse $env:LOCALAPPDATA\CheIME
```
