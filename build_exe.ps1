$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $Root

python -m pip install --upgrade pyinstaller Pillow
if (Test-Path build) { Remove-Item -Recurse -Force build }
if (Test-Path dist) { Remove-Item -Recurse -Force dist }

python -m PyInstaller --noconfirm --clean --windowed --name FuckExam --paths src --collect-all PIL src/FuckExam/__main__.py
Write-Host "Portable build ready: $Root\dist\FuckExam\FuckExam.exe"
Write-Host "Runtime data will be created beside the executable in FuckExamData."
