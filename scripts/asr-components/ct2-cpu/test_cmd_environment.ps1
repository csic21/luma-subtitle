param([Parameter(Mandatory=$true)][string]$Helper, [Parameter(Mandatory=$true)][string]$DeveloperBatch)
$ErrorActionPreference = 'Stop'
trap {
    # Never print the exception message, SET output or arbitrary native output.
    # These identifiers locate a failing operation without environment values.
    [Console]::Error.WriteLine(('CT2_QUOTING_FAILURE type={0} file={1} line={2} error_id={3}' -f
        $_.Exception.GetType().Name, [IO.Path]::GetFileName($_.InvocationInfo.ScriptName),
        $_.InvocationInfo.ScriptLineNumber, $_.FullyQualifiedErrorId))
    exit 1
}
. $Helper
Assert-CmdLiteral 'C:\Program Files\Microsoft Visual Studio\developer.cmd'
foreach ($invalid in @("bad`ncommand", "bad`rcommand", 'bad&command', 'bad|command', 'bad>file', 'bad<file', 'bad^command', '%PATH%', '!PATH!', 'bad"quote')) {
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
