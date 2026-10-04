param([Parameter(Mandatory=$true)][string]$Executable, [Parameter(Mandatory=$true)][string]$TestRoot)
$ErrorActionPreference = 'Stop'
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$TestRoot = [System.IO.Path]::GetFullPath($TestRoot)
New-Item -ItemType Directory -Path $TestRoot -Force | Out-Null
if (-not ('AgentHarborWakeCheck' -as [type])) {
  Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class AgentHarborWakeCheck {
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr window);
  [DllImport("user32.dll", EntryPoint="PostMessageW")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
  [DllImport("user32.dll", EntryPoint="GetWindowLongW")] public static extern int GetWindowLong(IntPtr window, int index);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
}
'@
}
function Assert-Condition([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Wait-Condition([scriptblock]$Condition, [string]$Message) {
  $deadline = [DateTime]::UtcNow.AddSeconds(5)
  while (-not (& $Condition)) {
    Assert-Condition ([DateTime]::UtcNow -lt $deadline) $Message
    Start-Sleep -Milliseconds 20
  }
}
function Test-Topmost([IntPtr]$Window) { return (([AgentHarborWakeCheck]::GetWindowLong($Window, -20) -band 8) -ne 0) }
$oldData = $env:AGENT_HARBOR_DATA_DIR
$oldCodex = $env:AGENT_HARBOR_CODEX_HOME
$ownedProcesses = [System.Collections.Generic.List[System.Diagnostics.Process]]::new()
try {
  $env:AGENT_HARBOR_DATA_DIR = Join-Path $TestRoot 'data'
  $env:AGENT_HARBOR_CODEX_HOME = Join-Path $TestRoot 'codex-home'
  New-Item -ItemType Directory -Path $env:AGENT_HARBOR_CODEX_HOME -Force | Out-Null
  $primary = Start-Process -FilePath $Executable -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $TestRoot 'startup.stdout.log') -RedirectStandardError (Join-Path $TestRoot 'startup.stderr.log')
  $ownedProcesses.Add($primary)
  Start-Sleep -Seconds 8
  $primary.Refresh()
  Assert-Condition (-not $primary.HasExited) 'Primary application exited during startup'
  $window = $primary.MainWindowHandle
  Assert-Condition ($window -ne [IntPtr]::Zero) 'Main window handle was not created'
  Assert-Condition ((Get-Item -LiteralPath (Join-Path $TestRoot 'startup.stderr.log')).Length -eq 0) 'Startup stderr was not empty'
  Assert-Condition (-not (Test-Topmost $window)) 'Startup left the window always on top'
  Write-Output 'PASS isolated EXE startup: alive, main window created, no stderr, not topmost'

  [void][AgentHarborWakeCheck]::ShowWindow($window, 6)
  for ($index = 0; $index -lt 3; $index++) {
    $secondary = Start-Process -FilePath $Executable -WindowStyle Hidden -PassThru
    $ownedProcesses.Add($secondary)
    Assert-Condition ($secondary.WaitForExit(5000)) 'Second instance failed to exit'
    Assert-Condition ($secondary.ExitCode -eq 0) 'Second instance exited with an error'
  }
  Wait-Condition { [AgentHarborWakeCheck]::IsWindowVisible($window) -and -not [AgentHarborWakeCheck]::IsIconic($window) } 'Second instance did not wake the main window'
  Start-Sleep -Milliseconds 500
  Assert-Condition (-not (Test-Topmost $window)) 'Repeated activation left a permanent topmost window'
  Write-Output 'PASS repeated second-instance activation restores a minimized window and clears its temporary topmost state'

  [void][AgentHarborWakeCheck]::PostMessage($window, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
  Wait-Condition { -not [AgentHarborWakeCheck]::IsWindowVisible($window) } 'Close request did not hide the main window'
  $secondary = Start-Process -FilePath $Executable -WindowStyle Hidden -PassThru
  $ownedProcesses.Add($secondary)
  Assert-Condition ($secondary.WaitForExit(5000)) 'Hide test second instance failed to exit'
  Wait-Condition { [AgentHarborWakeCheck]::IsWindowVisible($window) } 'Hidden main window was not revealed'
  [void][AgentHarborWakeCheck]::PostMessage($window, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
  Wait-Condition { -not [AgentHarborWakeCheck]::IsWindowVisible($window) } 'Close request during reveal did not hide the main window'
  Start-Sleep -Milliseconds 450
  Assert-Condition (-not [AgentHarborWakeCheck]::IsWindowVisible($window)) 'Delayed activation reopened a window hidden by the user'
  Assert-Condition (-not (Test-Topmost $window)) 'Hidden main window retained temporary topmost state'
  Write-Output 'PASS hiding the window during a reveal pulse stays hidden after the delayed callback'

  [void][AgentHarborWakeCheck]::SetWindowPos($window, [IntPtr]::new(-1), 0, 0, 0, 0, 0x13)
  $secondary = Start-Process -FilePath $Executable -WindowStyle Hidden -PassThru
  $ownedProcesses.Add($secondary)
  Assert-Condition ($secondary.WaitForExit(5000)) 'Topmost test second instance failed to exit'
  Wait-Condition { [AgentHarborWakeCheck]::IsWindowVisible($window) } 'Persistent topmost test did not reveal the window'
  Start-Sleep -Milliseconds 500
  Assert-Condition (Test-Topmost $window) 'Activation cleared a persistent topmost baseline'
  [void][AgentHarborWakeCheck]::SetWindowPos($window, [IntPtr]::new(-2), 0, 0, 0, 0, 0x13)
  Write-Output 'PASS activation preserves an existing persistent topmost baseline'

  [void][AgentHarborWakeCheck]::PostMessage($window, 0x112, [IntPtr]::new(0xF030), [IntPtr]::Zero)
  Wait-Condition { [AgentHarborWakeCheck]::IsZoomed($window) } 'Main window did not maximize'
  $secondary = Start-Process -FilePath $Executable -WindowStyle Hidden -PassThru
  $ownedProcesses.Add($secondary)
  Assert-Condition ($secondary.WaitForExit(5000)) 'Maximized test second instance failed to exit'
  Start-Sleep -Milliseconds 500
  Assert-Condition ([AgentHarborWakeCheck]::IsZoomed($window)) 'Activation restored a maximized window to normal size'
  Write-Output 'PASS activation preserves a maximized window size'
} finally {
  foreach ($process in $ownedProcesses) {
    if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit(5000) | Out-Null }
    $process.Dispose()
  }
  $env:AGENT_HARBOR_DATA_DIR = $oldData
  $env:AGENT_HARBOR_CODEX_HOME = $oldCodex
}
