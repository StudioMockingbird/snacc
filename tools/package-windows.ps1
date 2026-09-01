param(
    [Parameter(Mandatory = $true)]
    [string] $SnaccLicensePath,

    [string] $OutputDirectory,

    [switch] $IncludeDirectCompiler
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$metadata = cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) {
    throw 'Cargo metadata failed while determining the Snacc version.'
}
$snaccPackage = $metadata.packages | Where-Object { $_.name -eq 'cargo-snacc' } | Select-Object -First 1
if ($null -eq $snaccPackage) {
    throw 'Could not find the cargo-snacc package in workspace metadata.'
}
$version = $snaccPackage.version
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repositoryRoot "dist\snacc-$version-x86_64-pc-windows-msvc"
}

$output = [System.IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $output) {
    throw "Refusing to overwrite existing package path '$output'."
}
$license = Resolve-Path -LiteralPath $SnaccLicensePath
$releaseDirectory = Join-Path $repositoryRoot 'bin'
$llvmDll = Join-Path $releaseDirectory 'LLVM-C.dll'
$llvmLicense = Join-Path $releaseDirectory 'LICENSE-LLVM.txt'
$cargoSnacc = Join-Path $releaseDirectory 'cargo-snacc.exe'
$directCompiler = Join-Path $releaseDirectory 'snacc.exe'
$buildInfo = Join-Path $releaseDirectory 'build-info.json'

foreach ($required in @($cargoSnacc, $llvmDll, $llvmLicense, $buildInfo)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Required release input is missing: '$required'."
    }
}
$assembledBuild = Get-Content -LiteralPath $buildInfo -Raw | ConvertFrom-Json
if ($assembledBuild.profile -ne 'release' -or $assembledBuild.llvm_version -ne '22.1.8') {
    throw 'The root bin directory is not an assembled LLVM 22.1.8 release build. Run tools\build-snacc.ps1 -Release first.'
}
if ($IncludeDirectCompiler -and -not (Test-Path -LiteralPath $directCompiler -PathType Leaf)) {
    throw "The direct compiler was requested but is missing: '$directCompiler'."
}

$dumpbin = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
if ($null -eq $dumpbin) {
    throw 'dumpbin.exe is required to validate the LLVM runtime dependency closure. Run this script from an MSVC developer shell.'
}
$dependencies = & $dumpbin.Source /dependents $llvmDll
if ($LASTEXITCODE -ne 0) {
    throw 'dumpbin.exe could not inspect LLVM-C.dll.'
}
$systemDlls = @(
    'ADVAPI32.DLL', 'KERNEL32.DLL', 'NTDLL.DLL', 'OLE32.DLL', 'SHELL32.DLL'
)
$unexpected = $dependencies |
    ForEach-Object { if ($_ -match '^\s*([^\s]+\.dll)\s*$') { $Matches[1].ToUpperInvariant() } } |
    Where-Object {
        $_ -notin $systemDlls -and
        -not $_.StartsWith('API-MS-WIN-') -and
        -not $_.StartsWith('EXT-MS-WIN-')
    } |
    Sort-Object -Unique
if ($unexpected.Count -ne 0) {
    throw "LLVM-C.dll has unpackaged non-system dependencies: $($unexpected -join ', ')."
}

$parent = Split-Path -Parent $output
[System.IO.Directory]::CreateDirectory($parent) | Out-Null
$temporary = Join-Path $parent ('.snacc-package-' + [Guid]::NewGuid().ToString('N'))
[System.IO.Directory]::CreateDirectory($temporary) | Out-Null
try {
    Copy-Item -LiteralPath $cargoSnacc -Destination (Join-Path $temporary 'cargo-snacc.exe')
    if ($IncludeDirectCompiler) {
        Copy-Item -LiteralPath $directCompiler -Destination (Join-Path $temporary 'snacc.exe')
    }
    Copy-Item -LiteralPath $llvmDll -Destination (Join-Path $temporary 'LLVM-C.dll')
    Copy-Item -LiteralPath $license -Destination (Join-Path $temporary 'LICENSE-SNACC.txt')
    Copy-Item -LiteralPath $llvmLicense -Destination (Join-Path $temporary 'LICENSE-LLVM.txt')

    $files = Get-ChildItem -LiteralPath $temporary -File | Sort-Object Name
    $manifest = [ordered]@{
        schema = 1
        snacc_version = $version
        target = 'x86_64-pc-windows-msvc'
        llvm_version = '22.1.8'
        files = @($files | ForEach-Object {
            [ordered]@{
                path = $_.Name
                sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
                size = $_.Length
            }
        })
    }
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $temporary 'integrity.json') -Encoding utf8NoBOM

    $oldPath = $env:Path
    try {
        $env:Path = "$temporary;$oldPath"
        Push-Location $temporary
        try {
            & (Join-Path $temporary 'cargo-snacc.exe') doctor
            if ($LASTEXITCODE -ne 0) {
                throw 'The packaged cargo-snacc doctor check failed.'
            }
        } finally {
            Pop-Location
        }
    } finally {
        $env:Path = $oldPath
    }

    Move-Item -LiteralPath $temporary -Destination $output
    Write-Output "Created Windows package: $output"
} finally {
    if (Test-Path -LiteralPath $temporary) {
        Remove-Item -LiteralPath $temporary -Recurse -Force
    }
}
