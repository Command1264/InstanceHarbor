# Run: Invoke-Pester ./tests/resolve-npm.Tests.ps1   (plain assertions: works on Pester 3 and 5)
Describe 'resolve-npm.ps1' {
    $resolver = Join-Path (Split-Path -Parent $PSScriptRoot) 'resolve-npm.ps1'
    function Resolve-WithPath($dir, $name) {
        $old = $env:PATH
        try { $env:PATH = $dir; & $resolver -Search $name } finally { $env:PATH = $old }
    }
    It 'finds a launcher that has only an .exe extension' {
        $dir = Join-Path $TestDrive 'bin'; New-Item -ItemType Directory $dir | Out-Null
        Copy-Item (Join-Path $env:SystemRoot 'System32/whoami.exe') (Join-Path $dir 'fakenpm.exe')
        $got = Resolve-WithPath $dir 'fakenpm'
        if ($got -ne (Join-Path $dir 'fakenpm.exe')) { throw "unexpected: $got" }
    }
    It 'finds a launcher that has only a .cmd extension' {
        $dir = Join-Path $TestDrive 'cmd'; New-Item -ItemType Directory $dir | Out-Null
        Set-Content (Join-Path $dir 'fakenpm.cmd') '@echo off'
        $got = Resolve-WithPath $dir 'fakenpm'
        if ($got -ne (Join-Path $dir 'fakenpm.cmd')) { throw "unexpected: $got" }
    }
    It 'fails with an actionable message when npm is missing' {
        $message = try { & $resolver -Search 'no-such-npm-launcher'; '' } catch { $_.Exception.Message }
        if ($message -notlike '*找不到 npm*Node.js*PATH*') { throw "unexpected message: $message" }
    }
    It 'does not change PATH' {
        $before = $env:PATH
        & $resolver | Out-Null
        if ($env:PATH -ne $before) { throw 'PATH changed' }
    }
}
