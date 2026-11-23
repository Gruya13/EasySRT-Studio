<#
.SYNOPSIS
    Verifies NVIDIA CUDA 12.4 readiness and EasySRT Studio runtime installation on Windows.

.DESCRIPTION
    Standalone verification script intended to run on a physical Windows PC with an NVIDIA GPU
    after installing EasySRT Studio. Checks:
      1. nvidia-smi availability, GPU name, VRAM (MiB), and driver version >= 550.54 (CUDA 12.4).
      2. Presence of C:\Windows\System32\nvcuda.dll.
      3. EasySRT Studio install and runtime paths (%LOCALAPPDATA%\EasySRT Studio and
         %LOCALAPPDATA%\com.easysrt.studio), MSVC runtime DLLs, ffmpeg.exe, whisper-cli.exe,
         llama-server.exe, cublas64_12.dll, cudart64_12.dll, ggml-cuda.dll,
         ggml-silero-v6.2.0.bin, and ggml-large-v3-turbo-q5_0.bin.
      4. Live execution of ffmpeg.exe -version, whisper-cli.exe --help, and
         llama-server.exe --list-devices (confirming at least one CUDA device is enumerated).
      5. Prints a PASS/FAIL summary and exits with code 0 on pass or 1 on fail.

.PARAMETER AppDir
    Optional override for EasySRT Studio application install directory.
    Defaults to %LOCALAPPDATA%\EasySRT Studio.

.PARAMETER DataDir
    Optional override for EasySRT Studio local runtime/data directory.
    Defaults to %LOCALAPPDATA%\com.easysrt.studio.

.PARAMETER CommandTimeoutSeconds
    Timeout in seconds for each binary smoke-test command. Defaults to 30.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\scripts\test-windows-cuda.ps1
#>

[CmdletBinding()]
param(
    [string]$AppDir = $(if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "EasySRT Studio" } else { "EasySRT Studio" }),
    [string]$DataDir = $(if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "com.easysrt.studio" } else { "com.easysrt.studio" }),
    [int]$CommandTimeoutSeconds = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$MinDriverMajor = 550
$MinDriverMinor = 54
$MinVramMiB = 3500

$script:Results = New-Object System.Collections.Generic.List[object]

function Add-CheckResult {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][ValidateSet("PASS", "FAIL", "WARN")][string]$Status,
        [Parameter(Mandatory = $true)][string]$Detail
    )

    $entry = [PSCustomObject]@{
        Name   = $Name
        Status = $Status
        Detail = $Detail
    }
    $script:Results.Add($entry) | Out-Null

    $color = switch ($Status) {
        "PASS" { "Green" }
        "FAIL" { "Red" }
        "WARN" { "Yellow" }
    }
    Write-Host ("[{0}] {1}: {2}" -f $Status, $Name, $Detail) -ForegroundColor $color
}

function Write-Section {
    param([Parameter(Mandatory = $true)][string]$Title)
    Write-Host ""
    Write-Host ("=== {0} ===" -f $Title) -ForegroundColor Cyan
}

function Format-ByteSize {
    param([long]$Bytes)
    if ($Bytes -ge 1GB) {
        return ("{0:N2} GB" -f ($Bytes / 1GB))
    }
    if ($Bytes -ge 1MB) {
        return ("{0:N2} MB" -f ($Bytes / 1MB))
    }
    if ($Bytes -ge 1KB) {
        return ("{0:N1} KB" -f ($Bytes / 1KB))
    }
    return ("{0} B" -f $Bytes)
}

function Test-DriverVersionSupported {
    param([string]$RawVersion)
    if ([string]::IsNullOrWhiteSpace($RawVersion)) {
        return $false
    }
    $match = [regex]::Match($RawVersion.Trim(), '^(\d+)\.(\d+)')
    if (-not $match.Success) {
        return $false
    }
    $major = [int]$match.Groups[1].Value
    $minor = [int]$match.Groups[2].Value
    return ($major -gt $MinDriverMajor) -or ($major -eq $MinDriverMajor -and $minor -ge $MinDriverMinor)
}

function Format-ExitCodeHint {
    param([int]$ExitCode)
    switch ($ExitCode) {
        -1073741515 { return "0xC0000135 (STATUS_DLL_NOT_FOUND - missing MSVC or CUDA DLL)" }
        -1073741511 { return "0xC0000139 (STATUS_ENTRYPOINT_NOT_FOUND - incompatible DLL version)" }
        -1073741795 { return "0xC000001D (STATUS_ILLEGAL_INSTRUCTION - unsupported CPU instruction)" }
        -1073741819 { return "0xC0000005 (STATUS_ACCESS_VIOLATION)" }
        default { return ("exit code {0}" -f $ExitCode) }
    }
}

function Find-FileCandidate {
    param(
        [Parameter(Mandatory = $true)][string[]]$PreferredPaths,
        [string]$FallbackRoot = "",
        [string]$FileName = ""
    )

    foreach ($candidate in $PreferredPaths) {
        if (-not [string]::IsNullOrWhiteSpace($candidate) -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            return $candidate
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($FallbackRoot) -and -not [string]::IsNullOrWhiteSpace($FileName) -and (Test-Path -LiteralPath $FallbackRoot -PathType Container)) {
        $found = Get-ChildItem -LiteralPath $FallbackRoot -Filter $FileName -File -Recurse -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($null -ne $found) {
            return $found.FullName
        }
    }

    return $null
}

function Invoke-ProcessCaptured {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string]$Arguments,
        [string[]]$PrependPathDirs = @(),
        [int]$TimeoutSeconds = 30
    )

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $FilePath
    $psi.Arguments = $Arguments
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true

    $workDir = Split-Path -Parent $FilePath
    if (-not [string]::IsNullOrWhiteSpace($workDir) -and (Test-Path -LiteralPath $workDir -PathType Container)) {
        $psi.WorkingDirectory = $workDir
    }

    $extraDirs = New-Object System.Collections.Generic.List[string]
    foreach ($dir in $PrependPathDirs) {
        if (-not [string]::IsNullOrWhiteSpace($dir) -and (Test-Path -LiteralPath $dir -PathType Container)) {
            if (-not $extraDirs.Contains($dir)) {
                $extraDirs.Add($dir) | Out-Null
            }
        }
    }
    if ($extraDirs.Count -gt 0) {
        $currentPath = [Environment]::GetEnvironmentVariable("PATH")
        $prefix = ($extraDirs -join ";")
        $psi.EnvironmentVariables["PATH"] = if ([string]::IsNullOrEmpty($currentPath)) { $prefix } else { "$prefix;$currentPath" }
    }

    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo = $psi

    try {
        [void]$proc.Start()
        $stdoutTask = $proc.StandardOutput.ReadToEndAsync()
        $stderrTask = $proc.StandardError.ReadToEndAsync()

        $exited = $proc.WaitForExit($TimeoutSeconds * 1000)
        if (-not $exited) {
            try { $proc.Kill() } catch {}
            return [PSCustomObject]@{
                Success  = $false
                TimedOut = $true
                ExitCode = -1
                Stdout   = ""
                Stderr   = "Timed out after $TimeoutSeconds seconds"
            }
        }

        # Ensure async streams flush completely after process exits
        $proc.WaitForExit()
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()

        return [PSCustomObject]@{
            Success  = ($proc.ExitCode -eq 0)
            TimedOut = $false
            ExitCode = $proc.ExitCode
            Stdout   = $stdout
            Stderr   = $stderr
        }
    }
    catch {
        return [PSCustomObject]@{
            Success  = $false
            TimedOut = $false
            ExitCode = -1
            Stdout   = ""
            Stderr   = $_.Exception.Message
        }
    }
    finally {
        $proc.Dispose()
    }
}

Write-Host "============================================================" -ForegroundColor White
Write-Host " EasySRT Studio - Windows NVIDIA CUDA 12.4 Verification" -ForegroundColor White
Write-Host "============================================================" -ForegroundColor White
Write-Host ("App Directory : {0}" -f $AppDir)
Write-Host ("Data Directory: {0}" -f $DataDir)

# ---------------------------------------------------------------------------
# 1. Check nvidia-smi (GPU name, VRAM MiB, driver >= 550.54 for CUDA 12.4)
# ---------------------------------------------------------------------------
Write-Section "1. NVIDIA GPU & Driver (nvidia-smi)"

$nvidiaSmiPath = $null
$nvidiaSmiCmd = Get-Command "nvidia-smi" -ErrorAction SilentlyContinue
if ($null -ne $nvidiaSmiCmd) {
    $nvidiaSmiPath = $nvidiaSmiCmd.Source
}
else {
    $nvidiaSmiPath = Find-FileCandidate -PreferredPaths @(
        "C:\Windows\System32\nvidia-smi.exe",
        "C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe"
    )
}

if ($null -eq $nvidiaSmiPath) {
    Add-CheckResult -Name "nvidia-smi" -Status "FAIL" -Detail "nvidia-smi.exe not found on PATH or standard NVIDIA locations. Install NVIDIA Game Ready/Studio drivers (>= 550.54)."
}
else {
    $smiResult = Invoke-ProcessCaptured -FilePath $nvidiaSmiPath -Arguments "--query-gpu=name,memory.total,driver_version --format=csv,noheader,nounits" -TimeoutSeconds $CommandTimeoutSeconds
    if (-not $smiResult.Success) {
        $errDetail = if ($smiResult.Stderr) { $smiResult.Stderr.Trim() } else { Format-ExitCodeHint $smiResult.ExitCode }
        Add-CheckResult -Name "nvidia-smi" -Status "FAIL" -Detail "Execution failed ($errDetail)"
    }
    else {
        $lines = @($smiResult.Stdout -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        if ($lines.Count -eq 0) {
            Add-CheckResult -Name "nvidia-smi" -Status "FAIL" -Detail "nvidia-smi returned no GPU rows."
        }
        else {
            $anyDriverSupported = $false
            $gpuIndex = 0
            foreach ($line in $lines) {
                $parts = @($line -split ",")
                $gpuName = if ($parts.Count -ge 1) { $parts[0].Trim() } else { "Unknown NVIDIA GPU" }
                $vramRaw = if ($parts.Count -ge 2) { $parts[1].Trim() } else { "0" }
                $driverRaw = if ($parts.Count -ge 3) { $parts[2].Trim() } else { "" }

                $vramMiB = 0
                [void][int]::TryParse($vramRaw, [ref]$vramMiB)
                $driverOk = Test-DriverVersionSupported -RawVersion $driverRaw
                if ($driverOk) {
                    $anyDriverSupported = $true
                }

                $summary = "GPU #$gpuIndex '$gpuName' | VRAM: $vramMiB MiB | Driver: $driverRaw"
                if ($driverOk) {
                    Add-CheckResult -Name ("NVIDIA GPU #{0}" -f $gpuIndex) -Status "PASS" -Detail "$summary (CUDA 12.4 compatible: >= $MinDriverMajor.$MinDriverMinor)"
                }
                else {
                    Add-CheckResult -Name ("NVIDIA GPU #{0}" -f $gpuIndex) -Status "FAIL" -Detail "$summary (driver < $MinDriverMajor.$MinDriverMinor required for CUDA 12.4; please update NVIDIA driver)"
                }

                if ($vramMiB -gt 0 -and $vramMiB -lt $MinVramMiB) {
                    Add-CheckResult -Name ("NVIDIA VRAM #{0}" -f $gpuIndex) -Status "WARN" -Detail "VRAM ($vramMiB MiB) is below $MinVramMiB MiB threshold; EasySRT Studio defaults to CPU mode below 3500 MiB."
                }
                $gpuIndex++
            }

            if (-not $anyDriverSupported) {
                Add-CheckResult -Name "CUDA 12.4 Driver Requirement" -Status "FAIL" -Detail "No NVIDIA GPU met minimum driver version $MinDriverMajor.$MinDriverMinor."
            }
        }
    }
}

# ---------------------------------------------------------------------------
# 2. Check C:\Windows\System32\nvcuda.dll
# ---------------------------------------------------------------------------
Write-Section "2. CUDA Driver Library (nvcuda.dll)"

$nvcudaPath = "C:\Windows\System32\nvcuda.dll"
if (Test-Path -LiteralPath $nvcudaPath -PathType Leaf) {
    $nvcudaItem = Get-Item -LiteralPath $nvcudaPath
    $fileVersion = $nvcudaItem.VersionInfo.FileVersion
    $sizeStr = Format-ByteSize $nvcudaItem.Length
    Add-CheckResult -Name "nvcuda.dll" -Status "PASS" -Detail "$nvcudaPath found ($sizeStr, version: $fileVersion)"
}
else {
    Add-CheckResult -Name "nvcuda.dll" -Status "FAIL" -Detail "$nvcudaPath is missing. Reinstall NVIDIA graphics driver."
}

# ---------------------------------------------------------------------------
# 3. Check EasySRT Studio Installed Paths, Binaries, DLLs, and Models
# ---------------------------------------------------------------------------
Write-Section "3. EasySRT Studio Installation, DLLs & Models"

if (Test-Path -LiteralPath $AppDir -PathType Container) {
    $appExe = Join-Path $AppDir "EasySRT Studio.exe"
    if (Test-Path -LiteralPath $appExe -PathType Leaf) {
        Add-CheckResult -Name "App Install Dir" -Status "PASS" -Detail "$AppDir (found EasySRT Studio.exe)"
    }
    else {
        Add-CheckResult -Name "App Install Dir" -Status "WARN" -Detail "$AppDir exists, but EasySRT Studio.exe was not found directly inside."
    }
}
else {
    Add-CheckResult -Name "App Install Dir" -Status "FAIL" -Detail "$AppDir does not exist."
}

if (Test-Path -LiteralPath $DataDir -PathType Container) {
    Add-CheckResult -Name "Runtime Data Dir" -Status "PASS" -Detail "$DataDir exists"
}
else {
    Add-CheckResult -Name "Runtime Data Dir" -Status "FAIL" -Detail "$DataDir does not exist (run EasySRT Studio Setup Wizard first)."
}

# Resolve effective models directory (honoring modelsDirOverride in config.json if present)
$BinDir = Join-Path $DataDir "bin"
$ModelsDir = Join-Path $DataDir "models"
$ConfigPath = Join-Path $DataDir "config.json"
if (Test-Path -LiteralPath $ConfigPath -PathType Leaf) {
    try {
        $cfg = Get-Content -LiteralPath $ConfigPath -Raw | ConvertFrom-Json
        if ($null -ne $cfg.PSObject.Properties["modelsDirOverride"] -and -not [string]::IsNullOrWhiteSpace([string]$cfg.modelsDirOverride)) {
            $overrideModels = [string]$cfg.modelsDirOverride
            $ModelsDir = $overrideModels.Trim()
            Write-Host ("Using modelsDirOverride from config.json: {0}" -f $ModelsDir) -ForegroundColor Gray
        }
    }
    catch {
        Add-CheckResult -Name "config.json" -Status "WARN" -Detail "Could not parse $ConfigPath ($($_.Exception.Message)); using default models path."
    }
}

# Locate binaries
$ffmpegExe = Find-FileCandidate `
    -PreferredPaths @(
        (Join-Path $BinDir "ffmpeg\ffmpeg-9.1-essentials_build\bin\ffmpeg.exe"),
        (Join-Path $BinDir "ffmpeg\ffmpeg.exe")
    ) `
    -FallbackRoot (Join-Path $BinDir "ffmpeg") `
    -FileName "ffmpeg.exe"

$whisperExe = Find-FileCandidate `
    -PreferredPaths @(
        (Join-Path $BinDir "whisper\Release\whisper-cli.exe"),
        (Join-Path $BinDir "whisper\whisper-cli.exe")
    ) `
    -FallbackRoot (Join-Path $BinDir "whisper") `
    -FileName "whisper-cli.exe"

$llamaExe = Find-FileCandidate `
    -PreferredPaths @(
        (Join-Path $BinDir "llama\llama-server.exe")
    ) `
    -FallbackRoot (Join-Path $BinDir "llama") `
    -FileName "llama-server.exe"

$whisperDir = if ($null -ne $whisperExe) { Split-Path -Parent $whisperExe } else { Join-Path $BinDir "whisper\Release" }
$llamaDir = if ($null -ne $llamaExe) { Split-Path -Parent $llamaExe } else { Join-Path $BinDir "llama" }

# Check bundled MSVC runtime DLLs
$vcruntimeSearchDirs = @(
    (Join-Path $AppDir "resources\vcruntime"),
    (Join-Path $AppDir "vcruntime"),
    $AppDir
)
$bundledVcruntimeDir = $null
foreach ($candDir in $vcruntimeSearchDirs) {
    if ((Test-Path -LiteralPath $candDir -PathType Container) -and (Test-Path -LiteralPath (Join-Path $candDir "vcruntime140.dll") -PathType Leaf)) {
        $bundledVcruntimeDir = $candDir
        break
    }
}

$requiredMsvcDlls = @("vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll", "vcomp140.dll")
$missingMsvc = New-Object System.Collections.Generic.List[string]
$msvcSources = New-Object System.Collections.Generic.List[string]
foreach ($dll in $requiredMsvcDlls) {
    $foundDll = Find-FileCandidate -PreferredPaths @(
        $(if ($bundledVcruntimeDir) { Join-Path $bundledVcruntimeDir $dll } else { "" }),
        (Join-Path $whisperDir $dll),
        (Join-Path $llamaDir $dll),
        (Join-Path "C:\Windows\System32" $dll)
    )
    if ($null -eq $foundDll) {
        $missingMsvc.Add($dll) | Out-Null
    }
    else {
        $msvcSources.Add(("{0} ({1})" -f $dll, (Split-Path -Parent $foundDll))) | Out-Null
    }
}

if ($missingMsvc.Count -eq 0) {
    $bundledNote = if ($bundledVcruntimeDir) { "bundled dir: $bundledVcruntimeDir" } else { "resolved from binary/System32 paths" }
    Add-CheckResult -Name "MSVC Runtime DLLs" -Status "PASS" -Detail ("All required DLLs present ({0}; {1})" -f ($requiredMsvcDlls -join ", "), $bundledNote)
}
else {
    Add-CheckResult -Name "MSVC Runtime DLLs" -Status "FAIL" -Detail ("Missing required MSVC DLL(s): {0}" -f ($missingMsvc -join ", "))
}

# Report primary executables
if ($null -ne $ffmpegExe) {
    $sz = Format-ByteSize (Get-Item -LiteralPath $ffmpegExe).Length
    Add-CheckResult -Name "ffmpeg.exe" -Status "PASS" -Detail "$ffmpegExe ($sz)"
}
else {
    Add-CheckResult -Name "ffmpeg.exe" -Status "FAIL" -Detail "Not found under $(Join-Path $BinDir 'ffmpeg')"
}

if ($null -ne $whisperExe) {
    $sz = Format-ByteSize (Get-Item -LiteralPath $whisperExe).Length
    Add-CheckResult -Name "whisper-cli.exe" -Status "PASS" -Detail "$whisperExe ($sz)"
}
else {
    Add-CheckResult -Name "whisper-cli.exe" -Status "FAIL" -Detail "Not found under $(Join-Path $BinDir 'whisper')"
}

if ($null -ne $llamaExe) {
    $sz = Format-ByteSize (Get-Item -LiteralPath $llamaExe).Length
    Add-CheckResult -Name "llama-server.exe" -Status "PASS" -Detail "$llamaExe ($sz)"
}
else {
    Add-CheckResult -Name "llama-server.exe" -Status "FAIL" -Detail "Not found under $(Join-Path $BinDir 'llama')"
}

# Check CUDA 12.4 DLLs (cublas64_12.dll, cudart64_12.dll, ggml-cuda.dll) in whisper & llama folders
$cudaDllChecks = @(
    @{ Name = "cublas64_12.dll (whisper)"; Paths = @((Join-Path $whisperDir "cublas64_12.dll"), (Join-Path $BinDir "whisper\cublas64_12.dll")) },
    @{ Name = "cublas64_12.dll (llama)";   Paths = @((Join-Path $llamaDir "cublas64_12.dll")) },
    @{ Name = "cudart64_12.dll (whisper)"; Paths = @((Join-Path $whisperDir "cudart64_12.dll"), (Join-Path $BinDir "whisper\cudart64_12.dll")) },
    @{ Name = "cudart64_12.dll (llama)";   Paths = @((Join-Path $llamaDir "cudart64_12.dll")) },
    @{ Name = "ggml-cuda.dll (whisper)";   Paths = @((Join-Path $whisperDir "ggml-cuda.dll"), (Join-Path $BinDir "whisper\ggml-cuda.dll")) },
    @{ Name = "ggml-cuda.dll (llama)";     Paths = @((Join-Path $llamaDir "ggml-cuda.dll")) }
)

foreach ($check in $cudaDllChecks) {
    $hit = Find-FileCandidate -PreferredPaths $check.Paths
    if ($null -ne $hit) {
        $sz = Format-ByteSize (Get-Item -LiteralPath $hit).Length
        Add-CheckResult -Name $check.Name -Status "PASS" -Detail "$hit ($sz)"
    }
    else {
        Add-CheckResult -Name $check.Name -Status "FAIL" -Detail ("Missing (checked: {0})" -f ($check.Paths -join ", "))
    }
}

# Check required models (Silero VAD v6.2.0 and Whisper Large V3 Turbo Q5_0)
$vadModelPath = Find-FileCandidate `
    -PreferredPaths @((Join-Path $ModelsDir "vad\ggml-silero-v6.2.0.bin")) `
    -FallbackRoot $ModelsDir `
    -FileName "ggml-silero-v6.2.0.bin"

if ($null -ne $vadModelPath -and (Get-Item -LiteralPath $vadModelPath).Length -gt 0) {
    $sz = Format-ByteSize (Get-Item -LiteralPath $vadModelPath).Length
    Add-CheckResult -Name "ggml-silero-v6.2.0.bin" -Status "PASS" -Detail "$vadModelPath ($sz)"
}
else {
    Add-CheckResult -Name "ggml-silero-v6.2.0.bin" -Status "FAIL" -Detail "Missing or empty under $(Join-Path $ModelsDir 'vad')"
}

$whisperModelPath = Find-FileCandidate `
    -PreferredPaths @((Join-Path $ModelsDir "whisper\ggml-large-v3-turbo-q5_0.bin")) `
    -FallbackRoot $ModelsDir `
    -FileName "ggml-large-v3-turbo-q5_0.bin"

if ($null -ne $whisperModelPath -and (Get-Item -LiteralPath $whisperModelPath).Length -gt 0) {
    $sz = Format-ByteSize (Get-Item -LiteralPath $whisperModelPath).Length
    Add-CheckResult -Name "ggml-large-v3-turbo-q5_0.bin" -Status "PASS" -Detail "$whisperModelPath ($sz)"
}
else {
    Add-CheckResult -Name "ggml-large-v3-turbo-q5_0.bin" -Status "FAIL" -Detail "Missing or empty under $(Join-Path $ModelsDir 'whisper')"
}

# ---------------------------------------------------------------------------
# 4. Execute Binary Smoke Tests & Verify CUDA Enumeration
# ---------------------------------------------------------------------------
Write-Section "4. Binary Smoke Tests & CUDA Device Enumeration"

# 4a. ffmpeg.exe -version
if ($null -eq $ffmpegExe) {
    Add-CheckResult -Name "Run ffmpeg.exe -version" -Status "FAIL" -Detail "Skipped because ffmpeg.exe was not found."
}
else {
    $ffRes = Invoke-ProcessCaptured -FilePath $ffmpegExe -Arguments "-version" -PrependPathDirs @((Split-Path -Parent $ffmpegExe)) -TimeoutSeconds $CommandTimeoutSeconds
    $ffFirstLine = (($ffRes.Stdout + "`n" + $ffRes.Stderr) -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 1)
    if ([string]::IsNullOrWhiteSpace($ffFirstLine)) {
        $ffFirstLine = "(no output)"
    }
    if ($ffRes.Success -and ($ffRes.Stdout -match "(?i)ffmpeg version")) {
        Add-CheckResult -Name "Run ffmpeg.exe -version" -Status "PASS" -Detail $ffFirstLine
    }
    else {
        Add-CheckResult -Name "Run ffmpeg.exe -version" -Status "FAIL" -Detail ("Failed ({0}): {1}" -f (Format-ExitCodeHint $ffRes.ExitCode), $ffFirstLine)
    }
}

# 4b. whisper-cli.exe --help
if ($null -eq $whisperExe) {
    Add-CheckResult -Name "Run whisper-cli.exe --help" -Status "FAIL" -Detail "Skipped because whisper-cli.exe was not found."
}
else {
    $whisperPathDirs = @($whisperDir)
    if ($bundledVcruntimeDir) { $whisperPathDirs += $bundledVcruntimeDir }
    $whRes = Invoke-ProcessCaptured -FilePath $whisperExe -Arguments "--help" -PrependPathDirs $whisperPathDirs -TimeoutSeconds $CommandTimeoutSeconds
    $whCombined = ($whRes.Stdout + "`n" + $whRes.Stderr)
    if ($whRes.Success) {
        Add-CheckResult -Name "Run whisper-cli.exe --help" -Status "PASS" -Detail "Exited 0 cleanly (DLL loading verified)"
    }
    else {
        $hint = Format-ExitCodeHint $whRes.ExitCode
        $snippet = ($whCombined -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 2) -join " | "
        Add-CheckResult -Name "Run whisper-cli.exe --help" -Status "FAIL" -Detail ("Failed ({0}) {1}" -f $hint, $snippet)
    }
}

# 4c. llama-server.exe --list-devices (must enumerate an actual CUDA device, e.g. CUDA0: ...)
if ($null -eq $llamaExe) {
    Add-CheckResult -Name "Run llama-server.exe --list-devices" -Status "FAIL" -Detail "Skipped because llama-server.exe was not found."
}
else {
    $llamaPathDirs = @($llamaDir)
    if ($bundledVcruntimeDir) { $llamaPathDirs += $bundledVcruntimeDir }
    $llRes = Invoke-ProcessCaptured -FilePath $llamaExe -Arguments "--list-devices" -PrependPathDirs $llamaPathDirs -TimeoutSeconds $CommandTimeoutSeconds
    $llCombined = ($llRes.Stdout + "`n" + $llRes.Stderr)

    if (-not $llRes.Success) {
        $hint = Format-ExitCodeHint $llRes.ExitCode
        $snippet = ($llCombined -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 3) -join " | "
        Add-CheckResult -Name "Run llama-server.exe --list-devices" -Status "FAIL" -Detail ("Failed ({0}) {1}" -f $hint, $snippet)
    }
    else {
        $cudaLines = @($llCombined -split "`r?`n" | Where-Object { $_ -match "(?i)^\s*CUDA\d+\s*:" })
        if ($cudaLines.Count -gt 0) {
            $devicesText = ($cudaLines | ForEach-Object { $_.Trim() }) -join " ; "
            Add-CheckResult -Name "Run llama-server.exe --list-devices" -Status "PASS" -Detail ("CUDA device(s) enumerated: {0}" -f $devicesText)
        }
        else {
            $allLines = ($llCombined -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | ForEach-Object { $_.Trim() }) -join " ; "
            if ([string]::IsNullOrWhiteSpace($allLines)) { $allLines = "(no output)" }
            Add-CheckResult -Name "Run llama-server.exe --list-devices" -Status "FAIL" -Detail ("Exited 0 but NO CUDA device (CUDA0:) was enumerated. Output: {0}" -f $allLines)
        }
    }
}

# ---------------------------------------------------------------------------
# 5. Final PASS / FAIL Summary
# ---------------------------------------------------------------------------
$passCount = @($script:Results | Where-Object { $_.Status -eq "PASS" }).Count
$failCount = @($script:Results | Where-Object { $_.Status -eq "FAIL" }).Count
$warnCount = @($script:Results | Where-Object { $_.Status -eq "WARN" }).Count

Write-Host ""
Write-Host "============================================================" -ForegroundColor White
Write-Host (" SUMMARY: {0} PASS | {1} FAIL | {2} WARN" -f $passCount, $failCount, $warnCount) -ForegroundColor $(if ($failCount -eq 0) { "Green" } else { "Red" })
Write-Host "============================================================" -ForegroundColor White

if ($failCount -gt 0) {
    Write-Host "Failed checks:" -ForegroundColor Red
    foreach ($item in ($script:Results | Where-Object { $_.Status -eq "FAIL" })) {
        Write-Host ("  - {0}: {1}" -f $item.Name, $item.Detail) -ForegroundColor Red
    }
    Write-Host ""
    Write-Host "OVERALL RESULT: FAIL" -ForegroundColor Red
    exit 1
}

Write-Host "OVERALL RESULT: PASS (Windows NVIDIA CUDA 12.4 runtime is ready)" -ForegroundColor Green
exit 0
