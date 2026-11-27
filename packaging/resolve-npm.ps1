# Finds the npm launcher on PATH (npm.cmd from an installer, npm.exe from a version
# manager shim such as mise) and returns its full path. Reads PATH only; never changes it.
# -Search: command names to try in order (a test hook).
[CmdletBinding()]
param([string[]]$Search=@('npm'))
$ErrorActionPreference='Stop'
foreach($name in $Search){
    $found=Get-Command -Name $name -CommandType Application -ErrorAction SilentlyContinue|Select-Object -First 1
    if($found){return $found.Source}
}
throw '找不到 npm。請安裝 Node.js，或確認 npm（npm.cmd 或 npm.exe）在 PATH 上。'
