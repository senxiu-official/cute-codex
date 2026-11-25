param([Parameter(Mandatory = $true)][string]$AssetPath)
$ErrorActionPreference = 'Stop'
# Run the shipped control flow with synthetic startup facts and intercepted
# location/variable writes. No share or real process location is accessed.
$source = [IO.File]::ReadAllText($AssetPath).
    Replace('$ExecutionContext', '$testContext').
    Replace('[Environment]::CurrentDirectory', '$testCwd').
    Replace('Microsoft.PowerShell.Management\Set-Location', 'Set-TestLocation').
    Replace('Microsoft.PowerShell.Utility\Set-Variable', 'Set-TestVariable')
function Set-TestLocation {
    param([string]$LiteralPath, [string]$ErrorAction)
    $script:locationCalls += $LiteralPath
    if ($LiteralPath -notmatch '^[A-Za-z]:\\$') { throw 'Network location was probed' }
    $testContext.SessionState.Drive.Current.Root = $LiteralPath
}
function Set-TestVariable {
    param([string]$Name, [string]$Scope, $Value, [switch]$Force)
    $script:restored = $Value
}
foreach ($case in @(
    @{ cwd = '\\unreachable.invalid\share\project'; calls = 0; location = ''; restored = $false },
    @{ cwd = '\\?\UNC\unreachable.invalid\share\project'; calls = 0; location = ''; restored = $false },
    @{ cwd = 'D:\project'; calls = 1; location = 'project'; restored = $true },
    @{ cwd = 'C:\project'; calls = 0; location = 'project'; restored = $true }
)) {
    $testCwd = $case.cwd
    $script:locationCalls = @()
    $script:restored = $null
    $variables = New-Object PSObject
    $variables | Add-Member ScriptMethod GetValue { param($name) return $null }
    $drive = [pscustomobject]@{ Provider = [pscustomobject]@{ Name = 'FileSystem' }; Root = 'C:\'; CurrentLocation = '' }
    $testContext = [pscustomobject]@{ SessionState = [pscustomobject]@{
        LanguageMode = 'FullLanguage'; PSVariable = $variables
        Drive = [pscustomobject]@{ Current = $drive }
        Path = [pscustomobject]@{ CurrentLocation = $testCwd }
    } }
    & ([scriptblock]::Create($source))
    if ($locationCalls.Count -ne $case.calls) { throw "Unexpected location call for $testCwd" }
    if ($drive.CurrentLocation -ne $case.location) { throw "Unexpected drive location for $testCwd" }
    if (($null -ne $restored) -ne $case.restored) { throw "Unexpected PWD restoration for $testCwd" }
    if ($case.restored -and $restored -ne $testCwd) { throw "Incorrect PWD for $testCwd" }
}
Write-Output 'PASS: UNC, extended UNC, cross-drive and same-drive startup'
