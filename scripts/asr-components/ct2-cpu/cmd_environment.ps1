function Assert-CmdLiteral {
    param([Parameter(Mandatory=$true)][string]$Value)
    # CMD expansion/metacharacters, control characters and ambiguous encodings
    # are not valid in a reviewed toolchain path or the private temporary path.
    if ($Value -match '[\x00-\x1f\x7f-\uffff"&|<>^%!]') {
        throw 'Developer-shell input contains an unsafe CMD character.'
    }
}

function Set-DeveloperEnvironment {
    param(
        [Parameter(Mandatory=$true)][string]$DeveloperBatchPath,
        [Parameter(Mandatory=$true)][string[]]$Arguments
    )
    Assert-CmdLiteral $DeveloperBatchPath
    if (-not [IO.Path]::IsPathFullyQualified($DeveloperBatchPath) -or
        $DeveloperBatchPath.StartsWith('\\') -or -not (Test-Path -LiteralPath $DeveloperBatchPath -PathType Leaf)) {
        throw 'The reviewed developer batch file must be an existing local absolute path.'
    }
    foreach ($argument in $Arguments) {
        if ($argument -notmatch '^-[a-z_]+(?:=[a-zA-Z0-9.]+)?$') {
            throw 'Developer-shell option is not a bounded literal argument.'
        }
    }
    $batch = Join-Path ([IO.Path]::GetTempPath()) ('luma-ct2-vs-' + [Guid]::NewGuid().ToString('N') + '.cmd')
    Assert-CmdLiteral $batch
    $process = $null
    try {
        # CALL is the first command, so a path containing spaces never becomes
        # the leading nested-quote command string passed through PowerShell.
        $batchLines = @('@echo off', ('call "{0}" {1} >nul' -f $DeveloperBatchPath, ($Arguments -join ' ')),
                        'if errorlevel 1 exit /b %errorlevel%', 'set')
        [IO.File]::WriteAllLines($batch, $batchLines, [Text.Encoding]::ASCII)
        $start = [Diagnostics.ProcessStartInfo]::new()
        $start.FileName = $env:ComSpec
        $start.UseShellExecute = $false
        $start.RedirectStandardOutput = $true
        $start.RedirectStandardError = $true
        foreach ($argument in @('/d', '/c', 'call', $batch)) { $start.ArgumentList.Add($argument) }
        $process = [Diagnostics.Process]::new(); $process.StartInfo = $start
        if (-not $process.Start()) { throw 'Could not start the process-local developer shell.' }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(60000)) {
            $process.Kill($true); $process.WaitForExit()
            throw 'The process-local developer shell timed out.'
        }
        # SET output can contain credentials. Keep it in memory, never in logs,
        # report files, GITHUB_ENV or a temporary environment-output file.
        $lines = $stdoutTask.GetAwaiter().GetResult() -split "`r?`n"
        $null = $stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) { throw 'Pinned developer-shell setup failed; no global installation was attempted.' }
        foreach ($entry in $lines) {
            if ($entry -match '^([^=]+)=(.*)$') { [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], 'Process') }
        }
    } finally {
        if ($null -ne $process) { $process.Dispose() }
        if (Test-Path -LiteralPath $batch) { Remove-Item -LiteralPath $batch -Force }
        $lines = $null; $stdoutTask = $null; $stderrTask = $null
    }
}
