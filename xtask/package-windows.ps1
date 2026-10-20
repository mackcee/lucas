# Package only files required to run the Windows diagnostics build. Never include
# a developer's traces, settings, webcam data, PDBs or Cargo cache.
param([Parameter(Mandatory = $true)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$release = Join-Path $root 'target/x86_64-pc-windows-msvc/release'
$bundle = Join-Path $OutputDirectory 'lucas-windows-x64'
if (Test-Path $OutputDirectory) { throw 'Choose a new output directory to avoid packaging stale files' }
New-Item -ItemType Directory -Force $bundle | Out-Null
New-Item -ItemType Directory -Force (Join-Path $bundle 'models') | Out-Null
Copy-Item (Join-Path $release 'lucas.exe') $bundle
# ort rc.14 links ONNX Runtime statically but its Windows archive also requires
# DirectML.dll. MediaPipe selects CPU; UniGaze uses its separately installed CUDA child.
# Copy all runtime DLLs
# emitted by ort's copy-dylibs, including onnxruntime.dll if that layout changes.
$dlls = @(Get-ChildItem (Join-Path $release '*.dll') -File)
if ($dlls.Count -eq 0) { throw 'No native runtime DLLs found beside lucas.exe' }
$dlls | Copy-Item -Destination $bundle
Copy-Item (Join-Path $root 'models/face_detector.onnx') (Join-Path $bundle 'models')
Copy-Item (Join-Path $root 'models/face_landmarks_detector.onnx') (Join-Path $bundle 'models')
Copy-Item (Join-Path $root 'models/SHA256SUMS') (Join-Path $bundle 'models')
Copy-Item (Join-Path $root 'models/README.md') (Join-Path $bundle 'models')
# Explicit allowlist: never copy a developer's installed weights, venv or cache.
$unigaze = Join-Path $bundle 'models/unigaze'
New-Item -ItemType Directory -Force $unigaze | Out-Null
foreach ($name in @('setup.ps1', 'prepare.py', 'worker.py', 'assets.json', 'requirements-windows.lock', 'requirements.in', 'README.md')) {
    Copy-Item (Join-Path $root "models/unigaze/$name") $unigaze
}
# Public Apache-2.0 diagnostic fixture; never a user's camera image.
Copy-Item (Join-Path $root 'crates/lucas-gaze/tests/fixtures/portrait.jpg') $unigaze
foreach ($name in @('LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE')) {
    Copy-Item (Join-Path $root $name) $bundle
}
Copy-Item (Join-Path $root 'crates/lucas-app/licenses') $bundle -Recurse
Copy-Item (Join-Path $root 'docs/testing-on-windows.md') (Join-Path $bundle 'TESTING.md')
# App-local Visual C++ runtime: the unpacked build must not rely on Visual Studio
# or a preinstalled VC redistributable on the tester's machine.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products '*' -property installationPath
if ($LASTEXITCODE -ne 0 -or !$vs) { throw 'Visual Studio installation not found' }
$crt = Get-ChildItem (Join-Path $vs 'VC/Redist/MSVC/*/x64/Microsoft.VC*.CRT') -Directory |
    Sort-Object FullName -Descending | Select-Object -First 1
if (!$crt) { throw 'App-local Visual C++ redistributable not found' }
Get-ChildItem (Join-Path $crt.FullName '*.dll') -File | Copy-Item -Destination $bundle
$zip = Join-Path $OutputDirectory 'lucas-windows-x64.zip'
Compress-Archive -Path (Join-Path $bundle '*') -DestinationPath $zip -Force
# Test the ZIP, not the build folder; remove dev-tool PATH entries so a missing
# runtime dependency cannot be satisfied accidentally by the runner.
$unpacked = Join-Path $OutputDirectory 'unpacked-smoke'
Expand-Archive -Path $zip -DestinationPath $unpacked -Force
$oldPath = $env:PATH
try {
    $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
    $process = Start-Process -FilePath (Join-Path $unpacked 'lucas.exe') -ArgumentList '--check-runtime' -WorkingDirectory $unpacked -NoNewWindow -PassThru
    if (!$process.WaitForExit(30000)) {
        $process.Kill()
        throw 'Packaged runtime smoke check timed out'
    }
    if ($process.ExitCode -ne 0) { throw "Packaged runtime smoke check failed: $($process.ExitCode)" }
} finally { $env:PATH = $oldPath }
Write-Host "Ready-to-run archive: $zip"
