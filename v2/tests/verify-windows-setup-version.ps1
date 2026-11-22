# Checks the Setup's version order (update, repair, downgrade) with the NSIS 3.08 compiler.
# The test installer writes its results to a temporary folder and installs nothing.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Compiler)
$ErrorActionPreference = 'Stop'
$work = Join-Path ([IO.Path]::GetTempPath()) "ih-setup-version-$PID"
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $exe = Join-Path $work 'version-order.exe'
    $results = Join-Path $work 'results.txt'
    Push-Location $PSScriptRoot
    try {
        & $Compiler /NOCONFIG /V2 "/DOUTPUT=$exe" "/DRESULTS=$results" windows-setup-version.nsi
        if ($LASTEXITCODE -ne 0) { throw "makensis failed with exit code $LASTEXITCODE." }
    } finally { Pop-Location }
    $run = Start-Process -FilePath $exe -ArgumentList '/S' -Wait -PassThru
    if ($run.ExitCode -ne 0) { throw "The version test installer exited with $($run.ExitCode)." }
    $failures = @(Get-Content -LiteralPath $results | Where-Object {
        $parts = $_ -split ' '
        $parts.Count -ne 4 -or $parts[2] -cne $parts[3]
    })
    if ($failures.Count) { throw "Version order mismatch (installed setup expected actual):`n$($failures -join "`n")" }
    "Setup version order: $(@(Get-Content -LiteralPath $results).Count) cases passed."
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force
}
