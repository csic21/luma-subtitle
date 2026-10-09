param([Parameter(Mandatory=$true)][string]$Helper, [Parameter(Mandatory=$true)][string]$DeveloperBatch)
$ErrorActionPreference = 'Stop'
try {
    . $Helper
    Assert-CmdLiteral 'C:\Program Files\Microsoft Visual Studio\developer.cmd'
    # Exercise the complete printable ASCII class, including S/K/I case-fold peers.
    foreach ($code in 32..126) {
        $character = [string][char]$code
        if (-not '"&|<>^%!'.Contains($character)) { Assert-CmdLiteral $character }
    }
    foreach ($invalid in @("bad`ncommand", "bad`rcommand", 'bad&command', 'bad|command', 'bad>file', 'bad<file', 'bad^command', '%PATH%', '!PATH!', 'bad"quote', ([string][char]0x017f), ([string][char]0x212a), ([string][char]0x0130), ([string][char]0x007f))) {
        $rejected = $false
        try { Assert-CmdLiteral $invalid } catch { $rejected = $true }
        if (-not $rejected) { throw 'Unsafe CMD input was not rejected.' }
    }
    $before = @(Get-ChildItem -Path ([IO.Path]::GetTempPath()) -Filter 'luma-ct2-vs-*.cmd' | ForEach-Object Name)
    Set-DeveloperEnvironment -DeveloperBatchPath $DeveloperBatch -Arguments @('-no_logo', '-arch=x64')
    if ($env:LUMA_CT2_QUOTING_TEST -ne 'passed') { throw 'A developer path containing spaces did not execute correctly.' }
    $after = @(Get-ChildItem -Path ([IO.Path]::GetTempPath()) -Filter 'luma-ct2-vs-*.cmd' | ForEach-Object Name)
    if (($before -join "`n") -cne ($after -join "`n")) { throw 'Temporary batch file was not removed.' }
    Write-Output 'Native developer-shell quoting and input validation passed.'
} catch {
    # Never print the exception message, SET output or arbitrary native output.
    # These identifiers locate a failing operation without environment values.
    [Console]::Error.WriteLine(('CT2_QUOTING_FAILURE type={0} file={1} line={2}' -f
        $_.Exception.GetType().Name, [IO.Path]::GetFileName($_.InvocationInfo.ScriptName),
        $_.InvocationInfo.ScriptLineNumber))
    exit 1
}
