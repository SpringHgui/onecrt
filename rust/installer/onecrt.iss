; OneCRT Windows 安装包（Inno Setup 6 / 7）
;
; 正常走 rust/installer/build-installer.ps1，它会先构建 exe、把要装的文件
; 拷进 target\installer\stage，再把版本号与 stage 目录作为 /D 传进来。
;
; 手动编译：
;   ISCC.exe /DMyAppVersion=0.3.1 /DStageDir=..\target\installer\stage onecrt.iss

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif
#ifndef StageDir
  #define StageDir "..\target\installer\stage"
#endif

#define MyAppName "OneCRT"
#define MyAppExeName "onecrt.exe"
#define MyAppPublisher "SpringHgui"
#define MyAppUrl "https://github.com/SpringHgui/onecrt"

[Setup]
AppId={{43AE8C70-660C-4DCB-A2DF-90E72DA9580F}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppUrl}
AppSupportURL={#MyAppUrl}
AppUpdatesURL={#MyAppUrl}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
; 免签名的小工具按用户装，不弹 UAC：装到 %LOCALAPPDATA%\Programs\OneCRT
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\installer
OutputBaseFilename=onecrt-{#MyAppVersion}-windows-x64-setup
SetupIconFile=..\onecrt\assets\onecrt.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
DirExistsWarning=no
UsePreviousAppDir=yes
VersionInfoVersion={#MyAppVersion}
VersionInfoCompany={#MyAppPublisher}
VersionInfoProductName={#MyAppName}
VersionInfoDescription={#MyAppName} Setup

[Languages]
Name: "zh"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
