param(
    [Parameter(Mandatory=$true)][string]$Manifest,
    [Parameter(Mandatory=$true)][string]$WorkRoot,
    [Parameter(Mandatory=$true)][string]$Report
)
$ErrorActionPreference = 'Stop'
$expectedUrl = 'https://download.visualstudio.microsoft.com/download/pr/73aabf2e-9532-4f68-99f7-3247081a619c/CC0FF0EB1DC3F5188AE6300FAEF32BF5BEEBA4BDD6E8E445A9184072096B713B/VC_redist.x64.exe'
$expected = @{
    'installer' = @{ bytes=25635768; sha256='cc0ff0eb1dc3f5188ae6300faef32bf5beeba4bdd6e8e445a9184072096b713b' }
    'msvcp140.dll' = @{ bytes=557728; sha256='0f885b509a685d2bbfa652fed26b5fb31d88fbdab0a978c641d1c7b8aa460aa9' }
    'msvcp140_1.dll' = @{ bytes=35952; sha256='bfad5aef4c63a669e3c140655cdfdf395b6c979b400a447bd5dcb65ed8826c3d' }
}
$evidence = @{
    schema=1; source_sha=$env:SOURCE_SHA; passed=$false; files=@()
    source_url=$expectedUrl; installer_executed=$false; agreement_accepted=$false
    global_installation_performed=$false; binaries_uploaded=$false
    public_redistribution_authorized=$false
}
try {
    $work = [IO.Path]::GetFullPath($WorkRoot).TrimEnd([char]92, [char]47)
    $inputManifest = Get-Content -LiteralPath $Manifest -Raw | ConvertFrom-Json
    if ($inputManifest.schema -ne 1 -or $inputManifest.source_url -cne $expectedUrl -or
        [IO.Path]::GetFullPath($inputManifest.work_root).TrimEnd([char]92, [char]47) -ne $work -or
        @($inputManifest.files).Count -ne 3) {
        throw 'Unexpected direct-CRT proof manifest identity.'
    }
    foreach ($role in @('installer', 'msvcp140.dll', 'msvcp140_1.dll')) {
        $candidates = @($inputManifest.files | Where-Object { $_.role -ceq $role })
        if ($candidates.Count -ne 1) { throw 'Direct-CRT proof manifest must contain each fixed role exactly once.' }
        $entry = $candidates[0]
        $path = [IO.Path]::GetFullPath($entry.path)
        $relative = [IO.Path]::GetRelativePath($work, $path)
        if ([IO.Path]::IsPathFullyQualified($relative) -or $relative -eq '..' -or $relative.StartsWith('..\') -or $relative.StartsWith('../')) {
            throw 'Direct-CRT signature input escaped the private proof directory.'
        }
        $ancestor = $path
        while ($ancestor -ne $work) {
            $item = Get-Item -LiteralPath $ancestor
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Direct-CRT signature input traverses a reparse point.' }
            $ancestor = [IO.Path]::GetDirectoryName($ancestor)
        }
        $file = Get-Item -LiteralPath $path
        $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        $pin = $expected[$role]
        if ($file.Length -ne $pin.bytes -or $hash -cne $pin.sha256 -or $entry.bytes -ne $pin.bytes -or $entry.sha256 -cne $pin.sha256) {
            throw 'Direct-CRT signature input differs from the exact approved bytes.'
        }
        # Windows validates Authenticode; reading certificate metadata alone is
        # insufficient. The installer and extracted DLLs are never executed.
        $signature = Get-AuthenticodeSignature -LiteralPath $path
        $signer = $signature.SignerCertificate
        $evidence.files += @{
            role=$role; path=$relative; bytes=$file.Length; sha256=$hash
            signature_status=$signature.Status.ToString()
            signer_subject=$signer.Subject; signer_thumbprint=$signer.Thumbprint
            timestamp_subject=$signature.TimeStamperCertificate.Subject
            timestamp_thumbprint=$signature.TimeStamperCertificate.Thumbprint
            file_version=$file.VersionInfo.FileVersion; product_version=$file.VersionInfo.ProductVersion
        }
        if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
            $signer.Subject -notmatch '(?:^|, )O=Microsoft Corporation(?:,|$)' -or
            $signer.GetNameInfo([Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false) -notin @('Microsoft Corporation', 'Microsoft Windows Software Compatibility Publisher')) {
            throw 'Direct-CRT input does not have a valid Microsoft Authenticode signature.'
        }
        if ($file.VersionInfo.FileVersion -ne '14.44.35211.0') { throw 'Direct-CRT input has an unexpected file version.' }
    }
    $evidence.passed = $true
} catch {
    # Avoid arbitrary command output or environment data in diagnostics.
    $evidence.error_type = $_.Exception.GetType().Name
    $evidence.error_line = $_.InvocationInfo.ScriptLineNumber
    throw
} finally {
    $parent = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($Report))
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $evidence | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $Report -Encoding utf8
}
