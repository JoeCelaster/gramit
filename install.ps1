<#
.SYNOPSIS
    Installs gramit on Windows.

.DESCRIPTION
    Downloads the latest release, verifies its checksum, unpacks it into
    %LOCALAPPDATA%\Programs\gramit, and puts that directory on the user PATH.
    No administrator rights are needed at any point.

    Usual invocation:

        irm https://raw.githubusercontent.com/JoeCelaster/gramit/main/install.ps1 | iex

    `iex` cannot pass parameters, so configuration goes through the environment:

        $env:GRAMIT_VERSION        tag to install, e.g. v0.1.0 (default: latest)
        $env:GRAMIT_INSTALL_DIR    where the binaries go
        $env:GRAMIT_NO_MODIFY_PATH set to anything to skip the PATH edit

    To pass a switch instead, create the script block yourself:

        &([scriptblock]::Create((irm .../install.ps1))) -Uninstall
#>
[CmdletBinding()]
param(
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'

# Windows PowerShell 5.1 still negotiates SSLv3/TLS1.0 by default, which github.com
# refuses. PowerShell 7 already defaults to something sane.
try {
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch {
    # PowerShell 7+ on .NET Core: the property is obsolete and the default is fine.
}

$Repo    = 'JoeCelaster/gramit'
$Target  = 'x86_64-pc-windows-msvc'
$Archive = "gramit-$Target.zip"
$Version = if ($env:GRAMIT_VERSION) { $env:GRAMIT_VERSION } else { 'latest' }
$InstallDir = if ($env:GRAMIT_INSTALL_DIR) {
    $env:GRAMIT_INSTALL_DIR
} else {
    Join-Path $env:LOCALAPPDATA 'Programs\gramit'
}
# A trailing backslash would make the PATH comparisons below miss an existing entry
# and append a duplicate on every re-run.
$InstallDir = $InstallDir.TrimEnd('\')

# `gramit.exe` looks for `gramitd.exe` as a sibling before falling back to PATH
# (crates/gramit-cli/src/lifecycle.rs), so both always land in the same directory.
$Binaries = @('gramit.exe', 'gramitd.exe')

function Write-Step { param([string]$Text) Write-Host "  $Text" }
function Write-Note { param([string]$Text) Write-Host $Text }

function Get-AssetUrl {
    param([string]$Name)
    if ($Version -eq 'latest') {
        "https://github.com/$Repo/releases/latest/download/$Name"
    } else {
        "https://github.com/$Repo/releases/download/$Version/$Name"
    }
}

function Get-Asset {
    param([string]$Url, [string]$Destination)
    try {
        # The progress bar makes Invoke-WebRequest an order of magnitude slower on 5.1.
        $previous = $ProgressPreference
        $ProgressPreference = 'SilentlyContinue'
        Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing
    } catch {
        throw "download failed: $Url`n$($_.Exception.Message)"
    } finally {
        $ProgressPreference = $previous
    }
}

function Confirm-Checksum {
    param([string]$File, [string]$SumsFile, [string]$Name)

    $line = Select-String -Path $SumsFile -Pattern ([regex]::Escape($Name) + '$') |
        Select-Object -First 1
    if (-not $line) { throw "$Name is not listed in SHA256SUMS" }

    $expected = ($line.Line -split '\s+')[0].ToLowerInvariant()
    $actual   = (Get-FileHash -Path $File -Algorithm SHA256).Hash.ToLowerInvariant()

    if ($expected -ne $actual) {
        throw "checksum mismatch for $Name`n  expected $expected`n  got      $actual`nRefusing to install."
    }
    Write-Step 'checksum ok'
}

# Windows keeps a lock on a running .exe, so an upgrade over a live daemon fails with
# a confusing sharing violation. Ask it to stop, then insist.
function Stop-GramitDaemon {
    $cli = Join-Path $InstallDir 'gramit.exe'
    if (Test-Path $cli) {
        try { & $cli stop 2>&1 | Out-Null } catch { }
    }
    Get-Process -Name 'gramitd' -ErrorAction SilentlyContinue |
        Stop-Process -Force -ErrorAction SilentlyContinue
}

# Note on the API used here: reading the user PATH through [Environment] expands any
# %VARS% it contains, so writing it back stores them expanded. That is the same
# tradeoff every widely-used PowerShell installer makes, and the expanded path still
# resolves. Going through the registry to preserve REG_EXPAND_SZ would also mean
# broadcasting WM_SETTINGCHANGE by hand, which this API already does for us.
function Add-ToUserPath {
    param([string]$Directory)

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries  = @($userPath -split ';' | Where-Object { $_ })

    if (@($entries | ForEach-Object { $_.TrimEnd('\') }) -contains $Directory) {
        Write-Step "PATH already contains $Directory"
    } else {
        $updated = (@($entries) + $Directory) -join ';'
        [Environment]::SetEnvironmentVariable('Path', $updated, 'User')
        Write-Step "added $Directory to your user PATH"
    }

    # Make it usable in this window too, not just the next one.
    if (($env:Path -split ';') -notcontains $Directory) {
        $env:Path = "$env:Path;$Directory"
    }
}

function Remove-FromUserPath {
    param([string]$Directory)

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries  = @($userPath -split ';' |
        Where-Object { $_ -and $_.TrimEnd('\') -ne $Directory })
    [Environment]::SetEnvironmentVariable('Path', ($entries -join ';'), 'User')
    Write-Step "removed $Directory from your user PATH"
}

function Invoke-Uninstall {
    Write-Note 'Removing gramit...'
    Stop-GramitDaemon

    if (Test-Path $InstallDir) {
        Remove-Item -Path $InstallDir -Recurse -Force
        Write-Step "removed $InstallDir"
    } else {
        Write-Step "nothing found at $InstallDir"
    }

    Remove-FromUserPath -Directory $InstallDir

    Write-Note ''
    Write-Note 'Left in place on purpose:'
    Write-Step 'config  %APPDATA%\gramit\config.toml'
    Write-Step 'logs    %LOCALAPPDATA%\gramit'
}

function Invoke-Install {
    $arch = if ($env:PROCESSOR_ARCHITEW6432) {
        $env:PROCESSOR_ARCHITEW6432
    } else {
        $env:PROCESSOR_ARCHITECTURE
    }

    switch ($arch) {
        'AMD64' { }
        'ARM64' {
            Write-Note 'Note: there is no native ARM64 build yet. Installing the x64'
            Write-Note 'build, which Windows runs under emulation.'
        }
        default {
            throw "unsupported processor architecture: $arch. gramit needs 64-bit Windows."
        }
    }

    Write-Note "Installing gramit ($Version) for $Target"

    $work = Join-Path ([IO.Path]::GetTempPath()) ("gramit-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $work -Force | Out-Null

    try {
        $zip  = Join-Path $work $Archive
        $sums = Join-Path $work 'SHA256SUMS'

        Get-Asset -Url (Get-AssetUrl $Archive)     -Destination $zip
        Get-Asset -Url (Get-AssetUrl 'SHA256SUMS') -Destination $sums
        Confirm-Checksum -File $zip -SumsFile $sums -Name $Archive

        Expand-Archive -Path $zip -DestinationPath $work -Force

        Stop-GramitDaemon
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null

        foreach ($bin in $Binaries) {
            # The archive has a single top-level directory; flatten it away.
            $src = Join-Path $work "gramit-$Target\$bin"
            if (-not (Test-Path $src)) { throw "$bin is missing from the archive" }
            Copy-Item -Path $src -Destination (Join-Path $InstallDir $bin) -Force
            Write-Step "installed $(Join-Path $InstallDir $bin)"
        }
    } finally {
        Remove-Item -Path $work -Recurse -Force -ErrorAction SilentlyContinue
    }

    if (-not $env:GRAMIT_NO_MODIFY_PATH) {
        Add-ToUserPath -Directory $InstallDir
    } else {
        Write-Note ''
        Write-Note "PATH was left alone. Add this directory yourself: $InstallDir"
    }

    $reported = & (Join-Path $InstallDir 'gramit.exe') --version
    if (-not $reported) { throw "gramit.exe did not run after install" }
    Write-Step $reported

    Write-Note ''
    Write-Note 'gramit is installed. Next:'
    Write-Note ''
    Write-Step 'open a new terminal so PATH applies everywhere'
    Write-Step 'gramit setup             # tell gramit which backend to send text to'
    Write-Step 'gramit start'
    Write-Step 'gramit doctor'
    Write-Note ''
    Write-Note 'Then select text anywhere and press Ctrl+Alt+F.'
    Write-Note ''
    Write-Note 'gramit ships with no backend address: you choose where your text is sent,'
    Write-Note 'and it is saved only in your own config. See the README if you need to run one.'
}

if ($Uninstall -or $env:GRAMIT_UNINSTALL) {
    Invoke-Uninstall
} else {
    Invoke-Install
}
