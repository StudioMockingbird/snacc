param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $CargoArguments
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$llvmRoot = Join-Path $repositoryRoot 'vendor\clang+llvm-22.1.8-x86_64-pc-windows-msvc'
$llvmConfig = Join-Path $llvmRoot 'bin\llvm-config.exe'
$llvmDll = Join-Path $llvmRoot 'bin\LLVM-C.dll'

if (-not (Test-Path -LiteralPath $llvmConfig -PathType Leaf) -or
    -not (Test-Path -LiteralPath $llvmDll -PathType Leaf)) {
    throw "The vendored LLVM 22.1.8 distribution is incomplete at '$llvmRoot'."
}

$assemblesSnacc = $CargoArguments.Count -gt 0 -and $CargoArguments[0] -eq 'build'
if ($assemblesSnacc -and $CargoArguments -contains '--target') {
    throw 'Snacc build assembly does not support --target yet.'
}
if ($assemblesSnacc -and $CargoArguments -contains '--bin') {
    throw 'Selective --bin builds cannot produce a coherent Snacc bundle; build all binaries with tools\build-snacc.ps1.'
}

$env:LLVM_SYS_221_PREFIX = $llvmRoot
$env:Path = "$(Join-Path $llvmRoot 'bin');$env:Path"
& cargo @CargoArguments
$cargoExitCode = $LASTEXITCODE
if ($cargoExitCode -ne 0) {
    exit $cargoExitCode
}

if ($assemblesSnacc) {
    $profile = 'debug'
    if ($CargoArguments -contains '--release') {
        $profile = 'release'
    }
    $profileIndex = -1
    for ($index = 0; $index -lt $CargoArguments.Count; $index += 1) {
        if ($CargoArguments[$index] -eq '--profile') {
            $profileIndex = $index
            break
        }
    }
    if ($profileIndex -ge 0) {
        if ($profileIndex + 1 -ge $CargoArguments.Count) {
            throw '--profile requires a value.'
        }
        $profile = $CargoArguments[$profileIndex + 1]
        if ($profile -notin @('dev', 'release')) {
            throw "Snacc build assembly does not support profile '$profile'."
        }
        if ($profile -eq 'dev') {
            $profile = 'debug'
        }
    }

    $metadata = cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) {
        throw 'Cargo metadata failed after the Snacc build.'
    }
    $artifactDirectory = Join-Path $metadata.target_directory $profile
    $cargoSnacc = Join-Path $artifactDirectory 'cargo-snacc.exe'
    $directCompiler = Join-Path $artifactDirectory 'snacc.exe'
    foreach ($required in @($cargoSnacc, $directCompiler, $llvmDll)) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
            throw "A required Snacc bundle input is missing: '$required'."
        }
    }

    $outputDirectory = Join-Path $repositoryRoot 'bin'
    [System.IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
    $stagingDirectory = Join-Path $outputDirectory ('.staging-' + [Guid]::NewGuid().ToString('N'))
    [System.IO.Directory]::CreateDirectory($stagingDirectory) | Out-Null
    try {
        Copy-Item -LiteralPath $cargoSnacc -Destination (Join-Path $stagingDirectory 'cargo-snacc.exe')
        Copy-Item -LiteralPath $directCompiler -Destination (Join-Path $stagingDirectory 'snacc.exe')
        Copy-Item -LiteralPath $llvmDll -Destination (Join-Path $stagingDirectory 'LLVM-C.dll')
        Copy-Item -LiteralPath (Join-Path $llvmRoot 'include\llvm\Support\LICENSE.TXT') -Destination (Join-Path $stagingDirectory 'LICENSE-LLVM.txt')

        $snaccPackage = $metadata.packages | Where-Object { $_.name -eq 'cargo-snacc' } | Select-Object -First 1
        if ($null -eq $snaccPackage) {
            throw 'Could not find the cargo-snacc package in workspace metadata.'
        }
        $files = Get-ChildItem -LiteralPath $stagingDirectory -File | Sort-Object Name
        $manifest = [ordered]@{
            schema = 1
            snacc_version = $snaccPackage.version
            profile = $profile
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
        $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $stagingDirectory 'build-info.json') -Encoding utf8NoBOM

        foreach ($file in Get-ChildItem -LiteralPath $stagingDirectory -File) {
            $temporaryDestination = Join-Path $outputDirectory ('.' + $file.Name + '.' + [Guid]::NewGuid().ToString('N') + '.tmp')
            Copy-Item -LiteralPath $file.FullName -Destination $temporaryDestination
            Move-Item -LiteralPath $temporaryDestination -Destination (Join-Path $outputDirectory $file.Name) -Force
        }
    } finally {
        if (Test-Path -LiteralPath $stagingDirectory) {
            Remove-Item -LiteralPath $stagingDirectory -Recurse -Force
        }
    }
    Write-Output "Assembled Snacc $profile build in '$outputDirectory'."
}

exit 0
