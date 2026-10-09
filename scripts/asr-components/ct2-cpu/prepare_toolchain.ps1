param(
    [Parameter(Mandatory=$true)][string]$Reports,
    [Parameter(Mandatory=$true)][string]$Lock
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $Reports | Out-Null
$locked = Get-Content -Raw $Lock | ConvertFrom-Json
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path $vswhere)) { throw 'Official preinstalled Visual Studio discovery tool is missing.' }
$instances = & $vswhere -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -version '[17.0,18.0)' -format json | ConvertFrom-Json
$selected = @($instances | Where-Object { $_.installationVersion -eq $locked.toolchain.visual_studio_version })
$summary = @($instances | ForEach-Object { @{ version=$_.installationVersion; path=$_.installationPath; instance_id=$_.instanceId } })
$summary | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 (Join-Path $Reports 'visual-studio-instances.json')
if ($selected.Count -ne 1) { throw 'The pinned Visual Studio installation is unavailable; review inventory before repinning.' }
$vs = $selected[0]
$devcmd = Join-Path $vs.installationPath 'Common7\Tools\VsDevCmd.bat'
# Process-local developer environment only. No installer, registry edit or global PATH change.
$line = '""{0}" -no_logo -arch=x64 -host_arch=x64 -vcvars_ver={1} -winsdk={2} >nul && set"' -f $devcmd, $locked.toolchain.vc_tools_version, $locked.toolchain.windows_sdk_version
$lines = & $env:ComSpec /d /s /c $line
if ($LASTEXITCODE -ne 0) { throw 'The pinned compiler or SDK is not preinstalled.' }
foreach ($entry in $lines) {
    if ($entry -match '^([^=]+)=(.*)$') { [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], 'Process') }
}
$hashes = @{}
foreach ($name in @('cl.exe', 'link.exe', 'lib.exe', 'rc.exe', 'mt.exe')) {
    $tool = (Get-Command $name -ErrorAction Stop).Source
    $hashes[$name] = @{ sha256=(Get-FileHash -Algorithm SHA256 $tool).Hash.ToLowerInvariant(); file_version=(Get-Item $tool).VersionInfo.FileVersion }
}
$compiler = (Get-Command cl.exe).Source
$toolchain = @{
    schema=1; visual_studio_version=$vs.installationVersion; product_id=$vs.productId
    vc_tools_version=$env:VCToolsVersion.TrimEnd([char]92)
    windows_sdk_version=$env:WindowsSDKVersion.TrimEnd([char]92)
    compiler_version=(Get-Item $compiler).VersionInfo.FileVersion
    binary_hashes=$hashes; image_version=$env:ImageVersion; image_os=$env:ImageOS
    arch=$env:VSCMD_ARG_TGT_ARCH; host_arch=$env:VSCMD_ARG_HOST_ARCH
    source='Preinstalled Microsoft Visual Studio 2022 installation discovered by official vswhere'
    github_actions=($env:GITHUB_ACTIONS -eq 'true'); runner_environment=$env:RUNNER_ENVIRONMENT
}
$toolchain | ConvertTo-Json -Depth 12 | Set-Content -Encoding utf8 (Join-Path $Reports 'toolchain.json')
$redistRoot = $env:VCToolsRedistDir
if (-not $redistRoot -or -not (Test-Path $redistRoot)) { throw 'Official Visual Studio redistributable directory unavailable.' }
# Observe original named files only. Do not copy them into the runtime or upload them.
$candidates = @()
foreach ($folder in Get-ChildItem -Directory (Join-Path $redistRoot 'x64') -Filter 'Microsoft.VC*.CRT') {
    foreach ($file in Get-ChildItem -File $folder.FullName -Filter '*.dll') {
        $signature = Get-AuthenticodeSignature $file.FullName
        $candidates += @{
            filename=$file.Name; bytes=$file.Length
            sha256=(Get-FileHash -Algorithm SHA256 $file.FullName).Hash.ToLowerInvariant()
            source_path=$file.FullName; file_version=$file.VersionInfo.FileVersion
            product_version=$file.VersionInfo.ProductVersion
            signature_status=$signature.Status.ToString()
            signer_subject=$signature.SignerCertificate.Subject
            signer_thumbprint=$signature.SignerCertificate.Thumbprint
            timestamp_subject=$signature.TimeStamperCertificate.Subject
        }
    }
}
$report = @{
    schema=1; visual_studio_version=$vs.installationVersion; product_id=$vs.productId; vc_tools_version=$toolchain.vc_tools_version
    official_redist_directory=$redistRoot; candidates=$candidates
    copied_into_runtime=$false; binary_publication_authorized=$false; redistribution_review_passed=$false
    official_sources=@(
        'https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files?view=msvc-170',
        'https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution',
        'https://visualstudio.microsoft.com/license-terms/vs2022-ga-proenterprise/',
        'https://visualstudio.microsoft.com/license-terms/vs2022-cruntime/'
    )
    restrictions=@(
        'Microsoft runtime files are not permissively licensed open-source dependencies.',
        'Microsoft limits redistribution to licensed Visual Studio users under applicable license terms.',
        'A valid Authenticode signature and presence on REDIST list do not alone establish this project redistribution grant.',
        'No global redist installation, NumPy DLL renaming, static-CRT override, or binary publication is performed.'
    )
}
$report | ConvertTo-Json -Depth 12 | Set-Content -Encoding utf8 (Join-Path $Reports 'crt-candidates.json')
if ($candidates.Count -eq 0) { throw 'No official x64 CRT candidates found.' }
