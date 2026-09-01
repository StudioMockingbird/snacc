param(
    [switch] $Release,

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $CargoArguments
)

$ErrorActionPreference = 'Stop'
$arguments = @('build', '--workspace', '--bins')
if ($Release) {
    $arguments += '--release'
}
if ($null -ne $CargoArguments -and $CargoArguments.Count -gt 0) {
    $arguments += $CargoArguments
}

& (Join-Path $PSScriptRoot 'with-vendored-llvm.ps1') @arguments
exit $LASTEXITCODE
