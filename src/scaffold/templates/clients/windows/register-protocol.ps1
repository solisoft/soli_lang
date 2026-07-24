# Register {{SCHEME}}:// for the current user.
$exe = Join-Path $PSScriptRoot "bin\Release\net8.0-windows\{{SCHEME}}Shell.exe"
if (-not (Test-Path $exe)) {
  $exe = Join-Path $PSScriptRoot "{{SCHEME}}Shell.exe"
}
$base = "HKCU:\Software\Classes\{{SCHEME}}"
New-Item -Path $base -Force | Out-Null
Set-ItemProperty -Path $base -Name "(Default)" -Value "URL:{{APP_NAME}}"
Set-ItemProperty -Path $base -Name "URL Protocol" -Value ""
$cmd = Join-Path $base "shell\open\command"
New-Item -Path $cmd -Force | Out-Null
Set-ItemProperty -Path $cmd -Name "(Default)" -Value "`"$exe`" `"%1`""
Write-Host "Registered {{SCHEME}}:// -> $exe"
