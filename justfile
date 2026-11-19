# Smitten development tasks. Run `just` to list them.
#
# `just setup` installs everything needed to build and run Smitten on
# Windows, Linux or macOS. It needs just 1.44 or newer.

set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

python := if os_family() == "windows" { "python" } else { "python3" }

# List the recipes
default:
    @just --list

# Run one of the examples, e.g. `just run lantern`
run example *args:
    cargo run --example {{ example }} {{ args }}

# Install Rust, CMake, the MSVC C++ build tools and refresh the stable toolchain
[extension(".ps1")]
[script("powershell.exe", "-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File")]
[windows]
setup:
    $ErrorActionPreference = 'Stop'

    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        throw 'winget was not found. Install "App Installer" from the Microsoft Store, then run `just setup` again.'
    }

    function Install-WingetPackage([string] $Id, [string[]] $Extra = @()) {
        winget list --id $Id --exact --accept-source-agreements *> $null
        if ($LASTEXITCODE -eq 0) {
            Write-Host "$Id is already installed."
            return
        }
        Write-Host "Installing $Id..."
        winget install --id $Id --exact --source winget --accept-source-agreements --accept-package-agreements @Extra
        if ($LASTEXITCODE -ne 0) { throw "winget could not install $Id (exit code $LASTEXITCODE)." }
    }

    # SDL3 is compiled from source with MSVC, so the C++ build tools are needed.
    # Any Visual Studio 2022 edition that already has them is fine.
    $vsInstallerDir = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer'
    $vswhere = Join-Path $vsInstallerDir 'vswhere.exe'
    $vcTools = 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64'
    $hasVcTools = (Test-Path $vswhere) -and (& $vswhere -products * -requires $vcTools -property installationPath)
    if ($hasVcTools) {
        Write-Host 'The MSVC C++ build tools are already installed.'
    } else {
        $buildTools = if (Test-Path $vswhere) {
            & $vswhere -products Microsoft.VisualStudio.Product.BuildTools -property installationPath | Select-Object -First 1
        }
        if ($buildTools) {
            Write-Host 'Adding the C++ workload to the installed Visual Studio Build Tools...'
            $setupExe = Join-Path $vsInstallerDir 'setup.exe'
            $proc = Start-Process -FilePath $setupExe -Wait -PassThru -Verb RunAs -ArgumentList @(
                'modify', '--installPath', "`"$buildTools`"",
                '--add', 'Microsoft.VisualStudio.Workload.VCTools', '--includeRecommended',
                '--passive', '--norestart'
            )
            if ($proc.ExitCode -notin 0, 3010) { throw "The Visual Studio installer failed (exit code $($proc.ExitCode))." }
        } else {
            Install-WingetPackage 'Microsoft.VisualStudio.2022.BuildTools' @(
                '--override',
                '--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
            )
        }
    }

    # CMake drives the SDL3 build. The Build Tools ship a copy, but it is only
    # on the path inside the Developer PowerShell, so install one for every shell.
    if (Get-Command cmake -ErrorAction SilentlyContinue) {
        Write-Host 'CMake is already on the path.'
    } else {
        Install-WingetPackage 'Kitware.CMake' @('--custom', 'ADD_CMAKE_TO_PATH=System')
    }

    if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
        Install-WingetPackage 'Rustlang.Rustup'
    }

    # Pick up anything the installers just added to the path.
    $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
        [Environment]::GetEnvironmentVariable('Path', 'User') + ';' +
        (Join-Path $env:USERPROFILE '.cargo\bin')

    # The workspace uses the 2024 edition, so it needs Rust 1.85 or newer.
    rustup toolchain install stable --profile default
    if ($LASTEXITCODE -ne 0) { throw 'rustup could not install the stable toolchain.' }

    Write-Host ''
    Write-Host 'Done. Open a new terminal so the updated path takes effect, then try `just run lantern`.'

# Install Rust, CMake, a C compiler and SDL3's build dependencies
[linux]
[script("bash")]
setup:
    set -euo pipefail

    sudo=""
    if [ "$(id -u)" -ne 0 ]; then
        sudo="sudo"
    fi

    # SDL3 is compiled from source. These are its build dependencies from
    # https://wiki.libsdl.org/SDL3/README-linux#build-dependencies, which give
    # the X11, Wayland, ALSA, PulseAudio, PipeWire, GPU and gamepad backends.
    if command -v apt-get >/dev/null; then
        $sudo apt-get update
        $sudo apt-get install -y \
            build-essential git make pkg-config cmake ninja-build curl \
            libasound2-dev libpulse-dev libpipewire-0.3-dev libjack-jackd2-dev \
            libsndio-dev libaudio-dev \
            libx11-dev libxext-dev libxrandr-dev libxcursor-dev libxfixes-dev \
            libxi-dev libxss-dev libxtst-dev libxkbcommon-dev \
            libwayland-dev wayland-protocols libdecor-0-dev \
            libdrm-dev libgbm-dev libgl1-mesa-dev libgles-dev libegl-dev \
            libdbus-1-dev libibus-1.0-dev libudev-dev libusb-1.0-0-dev \
            libfribidi-dev libthai-dev liburing-dev
    elif command -v dnf >/dev/null; then
        $sudo dnf install -y \
            gcc gcc-c++ git-core make pkgconf-pkg-config cmake ninja-build curl \
            alsa-lib-devel pulseaudio-libs-devel pipewire-devel \
            pipewire-jack-audio-connection-kit-devel \
            libX11-devel libXext-devel libXrandr-devel libXcursor-devel libXfixes-devel \
            libXi-devel libXScrnSaver-devel libXtst-devel libxkbcommon-devel \
            wayland-devel wayland-protocols-devel libdecor-devel \
            libdrm-devel mesa-libgbm-devel mesa-libGL-devel mesa-libGLES-devel \
            mesa-libEGL-devel vulkan-devel \
            dbus-devel ibus-devel systemd-devel libusb1-devel \
            fribidi-devel libthai-devel liburing-devel
    elif command -v pacman >/dev/null; then
        $sudo pacman -S --needed --noconfirm \
            base-devel git pkgconf cmake ninja curl \
            alsa-lib libpulse pipewire sndio \
            libx11 libxext libxrandr libxcursor libxfixes libxi libxss libxtst \
            libxkbcommon wayland wayland-protocols libdecor \
            libdrm mesa libglvnd vulkan-headers \
            dbus ibus systemd-libs hidapi libusb fribidi libthai liburing
    else
        echo "No apt-get, dnf or pacman found. Install CMake, a C compiler and SDL3's" >&2
        echo "build dependencies by hand: https://wiki.libsdl.org/SDL3/README-linux#build-dependencies" >&2
        exit 1
    fi

    # The workspace uses the 2024 edition, so it needs Rust 1.85 or newer.
    export PATH="$HOME/.cargo/bin:$PATH"
    if ! command -v rustup >/dev/null; then
        if command -v cargo >/dev/null; then
            echo "Found cargo without rustup; make sure it is Rust 1.85 or newer."
            cargo --version
        else
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        fi
    else
        rustup toolchain install stable --profile default
    fi

    echo
    echo 'Done. Try `just run lantern`.'

# Install Rust, CMake and the Xcode command line tools
[macos]
[script("bash")]
setup:
    set -euo pipefail

    # SDL3 is compiled from source, which needs clang and CMake.
    if ! xcode-select -p >/dev/null 2>&1; then
        xcode-select --install
        echo "Finish the Xcode command line tools install, then run 'just setup' again."
        exit 1
    fi

    if ! command -v cmake >/dev/null; then
        if command -v brew >/dev/null; then
            brew install cmake
        else
            echo "Install CMake from https://cmake.org/download/ or Homebrew, then run 'just setup' again." >&2
            exit 1
        fi
    fi

    export PATH="$HOME/.cargo/bin:$PATH"
    if command -v rustup >/dev/null; then
        rustup toolchain install stable --profile default
    elif ! command -v cargo >/dev/null; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    fi

    echo
    echo 'Done. Try `just run lantern`.'

# Build the documentation site, with the API reference, into site/public. Needs Zola.
docs:
    {{ python }} site/tools/build.py

# Build the documentation site and serve it at http://127.0.0.1:1111
docs-serve:
    {{ python }} site/tools/build.py --serve
