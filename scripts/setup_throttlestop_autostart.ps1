# scripts/setup_throttlestop_autostart.ps1
# Tự động cấu hình ThrottleStop khởi động cùng Windows:
# - Chạy ngầm thu nhỏ xuống System Tray (StartMinimized).
# - Không bị hỏi bảng UAC (Highest Privileges).
# - Chạy được cả khi cắm sạc lẫn khi dùng pin (DisallowStartIfOnBatteries = false).

$tsExe = "D:\DownLoad\ThrottleStop_9.7\ThrottleStop.exe"
$tsDir = "D:\DownLoad\ThrottleStop_9.7"
$tsIni = "D:\DownLoad\ThrottleStop_9.7\ThrottleStop.ini"

if (-not (Test-Path $tsExe)) {
    Write-Error "Không tìm thấy file ThrottleStop.exe tại: $tsExe"
    exit 1
}

# 1. Cập nhật ThrottleStop.ini để tự động thu nhỏ xuống khay hệ thống, không bật cửa sổ to
if (Test-Path $tsIni) {
    $iniContent = Get-Content $tsIni
    $hasStartMin = $false
    $hasNotifArea = $false
    
    $newContent = foreach ($line in $iniContent) {
        if ($line -match "^StartMinimized=") {
            $hasStartMin = $true
            "StartMinimized=1"
        } elseif ($line -match "^NotificationArea=") {
            $hasNotifArea = $true
            "NotificationArea=1"
        } else {
            $line
        }
    }
    
    if (-not $hasStartMin) {
        $newContent += "StartMinimized=1"
    }
    if (-not $hasNotifArea) {
        $newContent += "NotificationArea=1"
    }
    
    $newContent | Set-Content $tsIni -Encoding Ascii
    Write-Output "[+] Đã cập nhật ThrottleStop.ini: StartMinimized=1, NotificationArea=1"
}

# 2. Tạo XML cấu hình Scheduled Task chuẩn Windows
$taskName = "ThrottleStop_AutoStart"
$currentUser = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name

$xml = @"
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Date>$([DateTime]::UtcNow.ToString("s"))</Date>
    <Author>$currentUser</Author>
    <Description>Auto-start ThrottleStop with Highest Privileges on Logon</Description>
    <URI>\$taskName</URI>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <Delay>PT3S</Delay>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>$tsExe</Command>
      <WorkingDirectory>$tsDir</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"@

$xmlPath = "C:\Users\default.LAPTOP-1V8CRH32\Documents\antigravity\fan-control\ts_task.xml"
$xml | Out-File -FilePath $xmlPath -Encoding Unicode

# 3. Đăng ký Scheduled Task vào hệ thống
schtasks /Create /TN $taskName /XML $xmlPath /F | Out-Null
if ($LASTEXITCODE -eq 0) {
    Write-Output "[+] Đã đăng ký thành công Scheduled Task: \$taskName"
} else {
    Write-Error "[-] Lỗi đăng ký Scheduled Task"
}

Remove-Item $xmlPath -ErrorAction SilentlyContinue
