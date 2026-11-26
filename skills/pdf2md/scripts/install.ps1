# pdf2md 바이너리와 OCR 모델을 <data-dir>\bin\ 아래에 설치한다(Windows x64 전용). install.sh와 같은 동작이다.
# 사용법: powershell -NoProfile -ExecutionPolicy Bypass -File install.ps1 <data-dir>
# - 이미 같은 버전이 설치되어 있고 모델 3개가 다 있으면 아무것도 받지 않는다(네트워크 없음).
# - 바이너리 압축은 같은 Release의 SHA256SUMS로, 모델은 아래 고정 해시로 검증한다.
# - 검증을 통과한 파일만 제자리로 옮기므로, 실패해도 기존 정상 파일은 그대로 남는다.
# Windows PowerShell 5.1에서도 돌도록 최신 문법(삼항 연산자, ??)을 쓰지 않는다.
param([Parameter(Mandatory = $true)][string]$DataDir)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue' # 5.1에서 진행 표시줄이 다운로드를 크게 느리게 만든다
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Version = '0.1.0' # Cargo.toml, .claude-plugin/plugin.json, install.sh와 같아야 한다(release CI가 확인)
$Repo = 'dktmzdf/Pdf2md'
# 테스트에서 로컬 서버로 바꿔 끼우기 위한 값. 평소에는 실제 주소를 쓴다.
$ReleaseBase = if ($env:PDF2MD_RELEASE_BASE) { $env:PDF2MD_RELEASE_BASE } else { "https://github.com/$Repo/releases/download/v$Version" }
$HfBase = if ($env:PDF2MD_HF_BASE) { $env:PDF2MD_HF_BASE } else { 'https://huggingface.co' }
$Os = if ($env:PDF2MD_OS) { $env:PDF2MD_OS } else { $env:OS }
$Arch = if ($env:PDF2MD_ARCH) { $env:PDF2MD_ARCH } elseif ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }

$DetCommit = 'e7bd63d9c674c96bd4fdab435af0efb383919c18'
$RecCommit = '5c6f574b8e2230adf4287b33e736d71b9fabd28e'

function Fail([string]$Message) {
    [Console]::Error.WriteLine("pdf2md install: $Message")
    exit 1
}

if (-not ($Os -eq 'Windows_NT' -and $Arch -eq 'AMD64')) {
    Fail "unsupported platform: $Os $Arch (supported: Linux x86_64, Windows x86_64)"
}
$Target = 'x86_64-pc-windows-msvc'
$Bin = Join-Path $DataDir 'bin'
$Models = Join-Path $Bin 'models'
New-Item -ItemType Directory -Force -Path $Models | Out-Null
$Tmp = Join-Path $DataDir (".install." + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Tmp | Out-Null

function Fetch([string]$Url, [string]$Out) {
    try { Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Out }
    catch { Fail "download failed: $Url ($($_.Exception.Message))" }
}
function Verify([string]$File, [string]$Expected) {
    $actual = (Get-FileHash -Algorithm SHA256 -Path $File).Hash.ToLowerInvariant()
    if ($actual -ne $Expected.ToLowerInvariant()) {
        Fail "checksum mismatch for $(Split-Path -Leaf $File): expected $Expected, got $actual"
    }
}

try {
    # 바이너리: 버전이 다르거나 없을 때만 받는다.
    $exe = Join-Path $Bin 'pdf2md.exe'
    $current = ''
    if (Test-Path $exe) { try { $current = (& $exe --version 2>$null | Out-String).Trim() } catch { $current = '' } }
    if ($current -ne "pdf2md $Version") {
        $archive = "pdf2md-$Target.zip"
        Fetch "$ReleaseBase/SHA256SUMS" (Join-Path $Tmp 'SHA256SUMS')
        Fetch "$ReleaseBase/$archive" (Join-Path $Tmp $archive)
        $expected = ''
        foreach ($line in Get-Content (Join-Path $Tmp 'SHA256SUMS')) {
            $parts = $line -split '\s+', 2
            if ($parts.Count -eq 2 -and ($parts[1] -eq $archive -or $parts[1] -eq "*$archive")) { $expected = $parts[0] }
        }
        if (-not $expected) { Fail "$archive is not listed in SHA256SUMS" }
        Verify (Join-Path $Tmp $archive) $expected
        $x = Join-Path $Tmp 'x'
        Expand-Archive -Path (Join-Path $Tmp $archive) -DestinationPath $x
        if (-not (Test-Path (Join-Path $x 'pdf2md.exe'))) { Fail "$archive does not contain pdf2md.exe" }
        Move-Item -Force (Join-Path $x 'pdf2md.exe') $exe
    }

    # 모델: 없는 파일만 받는다. 검증 후 옮기므로 있는 파일은 이미 검증된 것이다.
    $modelList = @(
        @('det.onnx', "$HfBase/deepghs/paddleocr/resolve/$DetCommit/det/ch_PP-OCRv4_det/model.onnx",
          '30a86f5731181461d08021402766601e4302a9b9b9666be8aff402696339cdff'),
        @('rec.onnx', "$HfBase/PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx/resolve/$RecCommit/inference.onnx",
          '92f0b7785e64fc9090106a241cf4c1eb97472824558272751b88a2a4476d3a08'),
        @('inference.yml', "$HfBase/PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx/resolve/$RecCommit/inference.yml",
          'f757fa1c40e99edcf27e9cce879b93eb2a51fa46f5ef39095689b8c37dd75998')
    )
    foreach ($m in $modelList) {
        $dst = Join-Path $Models $m[0]
        if (Test-Path $dst) { continue }
        $tmpFile = Join-Path $Tmp $m[0]
        Fetch $m[1] $tmpFile
        Verify $tmpFile $m[2]
        Move-Item -Force $tmpFile $dst
    }
}
finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $Tmp
}

Write-Output "pdf2md $Version ready: $exe"
