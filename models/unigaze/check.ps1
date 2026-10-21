# Reliable GUI-subsystem diagnostic invocation from interactive PowerShell.
# No camera or League access. Output is printed and retained locally.
param([string]$Executable = (Join-Path $PSScriptRoot '..\..\lucas.exe'))
$ErrorActionPreference = 'Stop'
$Executable = [IO.Path]::GetFullPath($Executable)
$models = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$directory = Join-Path (Join-Path $PSScriptRoot 'checks') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $directory -Force | Out-Null
$stdout = Join-Path $directory 'stdout.txt'
$stderr = Join-Path $directory 'stderr.txt'
$process = Start-Process -FilePath $Executable -ArgumentList @('--check-gaze-model', '--models', ('"' + $models + '"')) -Wait -NoNewWindow -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
Get-Content $stdout
Get-Content $stderr
Write-Host "Diagnostic logs: $directory"
exit $process.ExitCode
