# One-time, opt-in UniGaze install. No admin rights, camera or League access.
param([string]$InstallDirectory = $PSScriptRoot)
$ErrorActionPreference = 'Stop'
$InstallDirectory = [IO.Path]::GetFullPath($InstallDirectory)
New-Item -ItemType Directory -Force $InstallDirectory | Out-Null
# Marker is committed only after every download/dependency succeeded.
Remove-Item (Join-Path $InstallDirectory 'READY') -ErrorAction SilentlyContinue
foreach ($name in @('worker.py', 'prepare.py', 'check.ps1', 'assets.json', 'requirements-windows.lock')) {
    $src = Join-Path $PSScriptRoot $name
    $dst = Join-Path $InstallDirectory $name
    if ([IO.Path]::GetFullPath($src) -ne [IO.Path]::GetFullPath($dst)) { Copy-Item $src $dst -Force }
}
$bootstrap = Join-Path $InstallDirectory 'bootstrap'
New-Item -ItemType Directory -Force $bootstrap | Out-Null
$zip = Join-Path $bootstrap 'uv.zip'
$uvHash = '5d250c32d3604e28dbe18dc65c668ff628c53e00dde2c642576e831e4a60da64'
if (!(Test-Path $zip) -or (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant() -ne $uvHash) {
    Invoke-WebRequest -UseBasicParsing 'https://github.com/astral-sh/uv/releases/download/0.9.7/uv-x86_64-pc-windows-msvc.zip' -OutFile $zip
}
if ((Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant() -ne $uvHash) { throw 'uv SHA-256 mismatch; no code executed' }
Expand-Archive $zip $bootstrap -Force
$uv = Join-Path $bootstrap 'uv.exe'
$oldCache = $env:UV_CACHE_DIR
$oldPython = $env:UV_PYTHON_INSTALL_DIR
try {
    $env:UV_CACHE_DIR = Join-Path $InstallDirectory 'cache/uv'
    $env:UV_PYTHON_INSTALL_DIR = Join-Path $InstallDirectory 'python'
    & $uv python install --no-config --no-bin --no-registry 3.12.11
    if ($LASTEXITCODE -ne 0) { throw 'Python install failed' }
    & $uv venv --no-config --allow-existing --managed-python --python 3.12.11 (Join-Path $InstallDirectory '.venv')
    if ($LASTEXITCODE -ne 0) { throw 'UniGaze environment creation failed' }
    $python = Join-Path $InstallDirectory '.venv/Scripts/python.exe'
    & $uv pip sync --no-config --python $python --require-hashes --extra-index-url https://download.pytorch.org/whl/cu128 --index-strategy unsafe-best-match (Join-Path $InstallDirectory 'requirements-windows.lock')
    if ($LASTEXITCODE -ne 0) { throw 'Pinned CUDA dependency install failed' }
    & $python -I (Join-Path $InstallDirectory 'prepare.py') --root $InstallDirectory
    if ($LASTEXITCODE -ne 0) { throw 'UniGaze model download or SHA-256 verification failed' }
    Set-Content -Path (Join-Path $InstallDirectory 'READY') -Value 'lucas-unigaze-v1' -Encoding Ascii
} finally {
    $env:UV_CACHE_DIR = $oldCache
    $env:UV_PYTHON_INSTALL_DIR = $oldPython
}
Write-Host "UniGaze installed in $InstallDirectory. No camera has been opened."
Write-Host 'Next: powershell -NoProfile -ExecutionPolicy Bypass -File .\models\unigaze\check.ps1'
Write-Host 'Then: .\lucas.exe --calibrate (old MediaPipe calibrations cannot be reused).'
