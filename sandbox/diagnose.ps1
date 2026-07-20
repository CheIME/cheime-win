# CheIME TSF 注册诊断脚本
# 在沙箱内以管理员身份运行

param(
    [string]$DllPath = "$env:LOCALAPPDATA\CheIME\bin\cheime-tip.dll"
)

$ErrorActionPreference = 'Continue'

Write-Host "============================================" -ForegroundColor Cyan
Write-Host "  CheIME 注册诊断" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan

# 1. DLL 基本信息
Write-Host "`n[1] DLL 信息" -ForegroundColor Yellow
if (Test-Path $DllPath) {
    $dll = Get-Item $DllPath
    Write-Host "  路径: $DllPath"
    Write-Host "  大小: $($dll.Length / 1KB) KB"
    Write-Host "  修改时间: $($dll.LastWriteTime)"

    # 检查 DLL 位数
    $pe = Get-Content $DllPath -Encoding Byte -TotalCount 2
    if ($pe[0] -eq 0x4D -and $pe[1] -eq 0x5A) {
        Write-Host "  PE 头部: OK (MZ)"
    }
} else {
    Write-Host "  [错误] DLL 不存在: $DllPath"
}

# 2. regsvr32 注册测试
Write-Host "`n[2] regsvr32 注册测试" -ForegroundColor Yellow
$proc = Start-Process regsvr32.exe -ArgumentList "/s", $DllPath -Wait -PassThru -NoNewWindow
Write-Host "  regsvr32 退出码: $($proc.ExitCode)"
if ($proc.ExitCode -eq 0) {
    Write-Host "  状态: S_OK (成功)" -ForegroundColor Green
} else {
    Write-Host "  状态: 失败" -ForegroundColor Red
    switch ($proc.ExitCode) {
        0 { "成功" }
        1 { "ERROR_INVALID_FUNCTION / 加载 DLL 失败" }
        2 { "ERROR_FILE_NOT_FOUND" }
        3 { "ERROR_PATH_NOT_FOUND / DLL 依赖缺失" }
        5 { "ERROR_ACCESS_DENIED / 需要管理员" }
        default { "未知错误" }
    }
}

# 3. COM 注册表检查 (HKLM\Classes\CLSID)
Write-Host "`n[3] COM CLSID 注册表检查" -ForegroundColor Yellow
$clsid = "{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}"
$inprocKey = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Classes\CLSID\$clsid\InprocServer32", $false)
if ($inprocKey -ne $null) {
    $dllValue = $inprocKey.GetValue('')
    $threading = $inprocKey.GetValue('ThreadingModel')
    $inprocKey.Close()
    Write-Host "  HKLM\...\InprocServer32 默认值: $dllValue"
    Write-Host "  ThreadingModel: $threading"
} else {
    Write-Host "  [警告] HKLM CLSID 不存在" -ForegroundColor Red

    # 检查 HKCR (合并视图)
    $hkcrKey = [Microsoft.Win32.Registry]::ClassesRoot.OpenSubKey("CLSID\$clsid\InprocServer32", $false)
    if ($hkcrKey -ne $null) {
        $dllValue2 = $hkcrKey.GetValue('')
        $hkcrKey.Close()
        Write-Host "  HKCR\...\InprocServer32: $dllValue2 (HKCR 存在但 HKLM 不存在)"
    }
}

# 4. CTF TIP 注册检查
Write-Host "`n[4] CTF TIP 注册检查" -ForegroundColor Yellow
$ctfKey = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Microsoft\CTF\TIP\$clsid", $false)
if ($ctfKey -ne $null) {
    Write-Host "  CTF TIP key: 存在" -ForegroundColor Green
    $profileKey = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Microsoft\CTF\TIP\$clsid\LanguageProfile\0x00000804\{D7E2A3B4-C5F6-7890-ABCD-EF1234567890}", $false)
    if ($profileKey -ne $null) {
        $desc = $profileKey.GetValue('Description')
        Write-Host "  LanguageProfile Description: $desc"
        $profileKey.Close()
    } else {
        Write-Host "  [警告] LanguageProfile 子键不存在" -ForegroundColor Red
    }
    $ctfKey.Close()
} else {
    Write-Host "  [注意] CTF TIP key 不存在于 HKLM" -ForegroundColor Yellow
    Write-Host "  检查 HKCU 视图..."
    $ctfHkcu = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("SOFTWARE\Microsoft\CTF\TIP\$clsid", $false)
    if ($ctfHkcu -ne $null) {
        Write-Host "  HKCU CTF TIP key: 存在"
        $ctfHkcu.Close()
    } else {
        Write-Host "  [警告] HKCU CTF TIP key 也不存在" -ForegroundColor Red
    }
}

# 5. HKCU Enable 状态检查
Write-Host "`n[5] HKCU EnableLanguageProfile 状态" -ForegroundColor Yellow
$enablePath = "SOFTWARE\Microsoft\CTF\TIP\$clsid\LanguageProfile\0x00000804\{D7E2A3B4-C5F6-7890-ABCD-EF1234567890}"
$enableKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($enablePath, $false)
if ($enableKey -ne $null) {
    $enableVal = $enableKey.GetValue('Enable')
    Write-Host "  HKCU\$enablePath\Enable = $enableVal"
    if ($enableVal -eq 1) {
        Write-Host "  状态: 已启用" -ForegroundColor Green
    } elseif ($enableVal -eq 0) {
        Write-Host "  状态: 已禁用" -ForegroundColor Red
    } else {
        Write-Host "  [注意] Enable 值不存在或为空" -ForegroundColor Yellow
    }
    $enableKey.Close()
} else {
    Write-Host "  [注意] HKCU profile key 不存在" -ForegroundColor Yellow
    Write-Host "  这可能意味着 EnableLanguageProfile 未被调用或未生效"
}

# 6. 事件日志检查
Write-Host "`n[6] 最近 CheIME 相关事件日志" -ForegroundColor Yellow
try {
    $events = Get-WinEvent -FilterHashtable @{
        LogName = 'Application'
        ID = 1000,1001,1005,0
        StartTime = (Get-Date).AddHours(-1)
    } -ErrorAction Stop | Where-Object { $_.Message -match 'cheime|B5F1C9A8' } | Select-Object -First 5
    if ($events) {
        $events | ForEach-Object { Write-Host "  $($_.TimeCreated) Event $($_.Id): $($_.Message -replace '\s+',' ' -replace '.{200}$','...')" -ForegroundColor Red }
    } else {
        Write-Host "  无相关事件" -ForegroundColor Green
    }
} catch {
    Write-Host "  事件日志检查失败: $_"
}

# 7. 列出已注册的 TSF 输入法
Write-Host "`n[7] HKLM 已注册 TSF 输入法列表 (前 10 个)" -ForegroundColor Yellow
$tipRoot = "SOFTWARE\Microsoft\CTF\TIP"
$tipKey = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey($tipRoot, $false)
if ($tipKey -ne $null) {
    $count = 0
    foreach ($subKeyName in $tipKey.GetSubKeyNames()) {
        if ($count -ge 10) { break }
        $subKey = $tipKey.OpenSubKey($subKeyName)
        if ($subKey -ne $null) {
            $dspName = $subKey.GetValue('Display Description')
            if ($dspName) {
                Write-Host "  $subKeyName = $dspName"
                $count++
            }
            $subKey.Close()
        }
    }
    $tipKey.Close()
    Write-Host "  (显示 $count 个)"
}

# 8. 测试 COM 激活 (本地 probe 不需要注册)
Write-Host "`n[8] 建议" -ForegroundColor Yellow
Write-Host "  如果以上所有检查都通过，问题可能出在 TSF 激活 `$ActivateEx` 阶段。"
Write-Host "  在 Notepad 中："
Write-Host "    1. 打开 Notepad"
Write-Host "    2. 手动从语言栏选择 CheIME"
Write-Host "    3. 如果输入法立刻消失或无法打字，打开事件查看器检查崩溃"
Write-Host "    4. eventvwr.msc → Windows 日志 → 应用程序 → 过滤 ID 1000"

Write-Host "`n============================================" -ForegroundColor Cyan
Write-Host "  诊断完成" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
