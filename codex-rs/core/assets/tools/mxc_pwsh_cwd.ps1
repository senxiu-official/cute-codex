# PowerShell/PowerShell#27899: a denied ancestor can leave $PWD unset and the
# filesystem drive at its root. Use the process cwd without resolving it again.
# A successful startup or a profile that changed location has already set $PWD.
& {
    if ($ExecutionContext.SessionState.LanguageMode -ne 'FullLanguage') { return }
    if ($null -ne $ExecutionContext.SessionState.PSVariable.GetValue('global:PWD')) { return }
    $drive = $ExecutionContext.SessionState.Drive.Current
    $cwd = [Environment]::CurrentDirectory
    $root = [IO.Path]::GetPathRoot($cwd)
    # A failed startup can retain the default C: drive even when the process cwd
    # is on another drive. Select only a drive-letter root: resolving a UNC share
    # could block on network IO and leave the shell at the share root.
    if ($root -match '^[A-Za-z]:\\$' -and
        $drive.Provider.Name -eq 'FileSystem' -and !$drive.CurrentLocation -and $drive.Root -ne $root) {
        try { Microsoft.PowerShell.Management\Set-Location -LiteralPath $root -ErrorAction Stop }
        catch { return }
        $drive = $ExecutionContext.SessionState.Drive.Current
    }
    if ($drive.Provider.Name -eq 'FileSystem' -and !$drive.CurrentLocation -and
        $drive.Root -eq [IO.Path]::GetPathRoot($cwd) -and $cwd.Length -gt $drive.Root.Length) {
        $drive.CurrentLocation = $cwd.Substring($drive.Root.Length)
        Microsoft.PowerShell.Utility\Set-Variable -Name PWD -Scope Global -Value $ExecutionContext.SessionState.Path.CurrentLocation -Force
    }
}
