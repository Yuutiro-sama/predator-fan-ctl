; installer/predator-fan-ctl.iss
#define MyAppName "Predator Fan Controller"
#define MyAppVersion "1.4.0"
#define MyAppPublisher "Predator Thermal Engineering"
#define MyAppExeName "predator-fan-ctl.exe"

[Setup]
AppId={{8F9E4A21-7C6B-4D3E-9A12-5B8E3F204D99}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} v{#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\PredatorFanCtl
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
; Yêu cầu quyền Administrator khi chạy Setup
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Xuất file Setup ra thư mục dist/
OutputDir=..\dist
OutputBaseFilename=predator-fan-ctl-setup-v{#MyAppVersion}
; Nhúng icon Predator cho chính file Setup.exe và trong Control Panel (Installed Apps)
SetupIconFile=..\assets\predator.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
UninstallDisplayName={#MyAppName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
CloseApplications=force

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "autostart"; Description: "Start automatically with Windows (Highest Privileges via Task Scheduler)"; GroupDescription: "System Integration:"; Flags: checkedonce

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\predator.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\inpoutx64.dll"; DestDir: "{app}"; Flags: ignoreversion
; Chỉ chép config.toml mặc định nếu máy người dùng chưa có sẵn file cấu hình
Source: "..\config.toml"; DestDir: "{app}"; Flags: onlyifdoesntexist uninsneveruninstall

[Icons]
; 1. Start Menu Shortcut (Đặt trực tiếp trong Common Programs để Windows Search index ngay lập tức)
Name: "{commonprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; IconFilename: "{app}\{#MyAppExeName}"; Comment: "Acer Predator Ultra-Lightweight Fan Controller"
; 2. Desktop Shortcut
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; IconFilename: "{app}\{#MyAppExeName}"; Tasks: desktopicon; Comment: "Acer Predator Ultra-Lightweight Fan Controller"

[Run]
; Đăng ký Task Scheduler chạy quyền HIGHEST lúc Logon nếu người dùng tick chọn autostart
Filename: "schtasks.exe"; Parameters: "/Create /F /TN ""PredatorFanController"" /TR ""\""{app}\{#MyAppExeName}\"""" /SC ONLOGON /RL HIGHEST"; Flags: runhidden waituntilterminated; Tasks: autostart
; Tùy chọn khởi chạy ngay sau khi cài xong
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent shellexec

[UninstallRun]
; Dừng tiến trình đang chạy và xóa Scheduled Task + Windows Service (nếu có) trước khi xóa file
Filename: "taskkill.exe"; Parameters: "/F /IM {#MyAppExeName}"; Flags: runhidden; RunOnceId: "KillPredatorFanCtl"
Filename: "taskkill.exe"; Parameters: "/F /IM predator-fan-ctl-running.exe"; Flags: runhidden; RunOnceId: "KillPredatorFanCtlRunning"
Filename: "schtasks.exe"; Parameters: "/Delete /F /TN ""PredatorFanController"""; Flags: runhidden; RunOnceId: "DelScheduledTask"
Filename: "sc.exe"; Parameters: "stop PredatorFanSvc"; Flags: runhidden; RunOnceId: "StopWinSvc"
Filename: "sc.exe"; Parameters: "delete PredatorFanSvc"; Flags: runhidden; RunOnceId: "DelWinSvc"

[UninstallDelete]
; Xóa sạch config.toml và thư mục khi gỡ cài đặt hoàn toàn
Type: files; Name: "{app}\config.toml"
Type: dirifempty; Name: "{app}"

[Code]
// Tự động tắt tiến trình đang chạy trước khi cài đè bản mới để không bị lỗi khóa file EXE
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  Exec('taskkill.exe', '/F /IM {#MyAppExeName}', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec('taskkill.exe', '/F /IM predator-fan-ctl-running.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := '';
end;

// Tự động tắt tiến trình trước khi gỡ cài đặt
function InitializeUninstall(): Boolean;
var
  ResultCode: Integer;
begin
  Exec('taskkill.exe', '/F /IM {#MyAppExeName}', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec('taskkill.exe', '/F /IM predator-fan-ctl-running.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := True;
end;
