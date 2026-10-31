param([switch]$DebugBuild)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
Push-Location $projectRoot
try {
    $cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
    $cargoBinary = if ($cargoCommand) { $cargoCommand.Source } else { Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
    if (!(Test-Path -LiteralPath $cargoBinary -PathType Leaf)) { throw 'Install Rust using rustup first.' }

    $shaderSource = Join-Path $projectRoot 'crates\hl2-bevy\assets'
    if (!(Test-Path -LiteralPath $shaderSource -PathType Container)) { throw 'Bevy shader assets are missing from crates\hl2-bevy\assets.' }
    if (!(Get-ChildItem -LiteralPath $shaderSource -File -Recurse)) { throw 'Bevy shader assets are empty.' }

    $profileName = if ($DebugBuild) { 'debug' } else { 'release' }
    $cargoArguments = @('build', '--locked', '-p', 'hl2-app', '--bin', 'hl2-bevy')
    if (!$DebugBuild) { $cargoArguments += '--release' }
    & $cargoBinary @cargoArguments
    if ($LASTEXITCODE -ne 0) { throw 'Cargo Bevy build failed.' }

    $targetRoot = if ($env:CARGO_TARGET_DIR) { [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR) } else { Join-Path $projectRoot 'target' }
    if ($env:CARGO_BUILD_TARGET) {
        $targetRoot = Join-Path $targetRoot ([IO.Path]::GetFileNameWithoutExtension($env:CARGO_BUILD_TARGET))
    }
    $sourceExecutable = Join-Path $targetRoot "$profileName\hl2-bevy.exe"
    if (!(Test-Path -LiteralPath $sourceExecutable -PathType Leaf)) { throw "Built executable not found: $sourceExecutable" }

    $packageRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot 'bin'))
    $shaderDestination = [IO.Path]::GetFullPath((Join-Path $packageRoot 'bevy-assets'))
    $workspacePrefix = $projectRoot.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    if (!$packageRoot.StartsWith($workspacePrefix, [StringComparison]::OrdinalIgnoreCase) -or
        !($shaderDestination -eq (Join-Path $packageRoot 'bevy-assets'))) {
        throw 'Bevy package destination is outside the project.'
    }
    foreach ($packagePath in @($packageRoot, $shaderDestination)) {
        if ((Test-Path -LiteralPath $packagePath) -and
            ((Get-Item -LiteralPath $packagePath -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Bevy package destination must not be a link: $packagePath"
        }
    }
    New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
    # Only replace this verified generated directory, so removed shaders cannot linger.
    if (Test-Path -LiteralPath $shaderDestination) { Remove-Item -LiteralPath $shaderDestination -Recurse -Force }
    Copy-Item -LiteralPath $shaderSource -Destination $shaderDestination -Recurse -Force
    Copy-Item -LiteralPath (Join-Path $projectRoot 'plugins\hl2-advanced-options\assets\shaders\weapon_blur.wgsl') -Destination (Join-Path $shaderDestination 'shaders\weapon_blur.wgsl') -Force
    $packageExecutable = Join-Path $packageRoot 'hl2-bevy.exe'
    Copy-Item -LiteralPath $sourceExecutable -Destination $packageExecutable -Force

    $shaderHashes = @(Get-ChildItem -LiteralPath $shaderDestination -File -Recurse | Sort-Object FullName | ForEach-Object {
        [ordered]@{
            path = 'bin/bevy-assets/' + $_.FullName.Substring($shaderDestination.Length + 1).Replace('\', '/')
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
        }
    })
    $packageMetadata = [ordered]@{
        built_utc = [DateTime]::UtcNow.ToString('o')
        profile = $profileName
        executable = 'bin/hl2-bevy.exe'
        sha256 = (Get-FileHash -LiteralPath $packageExecutable -Algorithm SHA256).Hash
        assets = $shaderHashes
    } | ConvertTo-Json -Depth 4
    $metadataFile = Join-Path $packageRoot 'build-bevy-info.json'
    $temporaryMetadataFile = Join-Path $packageRoot ('.build-bevy-info-' + [Guid]::NewGuid().ToString('N') + '.tmp')
    try {
        $packageMetadata | Set-Content -LiteralPath $temporaryMetadataFile -Encoding UTF8
        for ($attempt = 0; $attempt -lt 5; $attempt++) {
            try {
                Move-Item -LiteralPath $temporaryMetadataFile -Destination $metadataFile -Force
                break
            } catch {
                if ($attempt -eq 4) { throw }
                Start-Sleep -Milliseconds 100
            }
        }
    } finally {
        if (Test-Path -LiteralPath $temporaryMetadataFile) { Remove-Item -LiteralPath $temporaryMetadataFile -Force }
    }
    Write-Host 'Built bin\hl2-bevy.exe with bin\bevy-assets. Run launch-bevy.cmd.'
} finally { Pop-Location }
