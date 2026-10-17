The changelog can be found on the [releases page](https://github.com/openai/codex/releases).

## Unreleased

- cute-codex: under the unelevated (restricted-token) Windows sandbox, a shell command whose PowerShell is the Microsoft Store build (`C:\Program Files\WindowsApps\Microsoft.PowerShell_*\pwsh.exe` or its `WindowsApps\pwsh.exe` alias) failed to start (`CreateProcessAsUserW` access denied, tool `exit -1`). It now gets the same unpackaged fallback as the elevated and MXC backends (an installed `pwsh.exe`, else Windows PowerShell); without a sandbox the user's PATH choice is unchanged.
- Fix missing command lifecycle events when unified exec cannot create a process, preserving approval rejection and sandbox retry behavior. Thanks @Marvinthebored ([#44557](https://github.com/openai/codex/issues/44557)).
