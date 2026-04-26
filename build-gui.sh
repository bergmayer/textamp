#!/bin/bash
# Build script for the textamp GUI binary (macOS and Linux).
# Usage: ./build-gui.sh [--makepackage | --clean | --no-native-menus | --help]

set -e

PKG_NAME="textamp"
PKG_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
PKG_DESC="A keyboard-driven Plex Music client (desktop GUI)"
PKG_LICENSE="MIT"
PKG_AUTHOR="John Bergmayer"
PKG_MAINTAINER="John Bergmayer"
BINARY_GUI="target/release/${PKG_NAME}-gui"

# Enable native menu bars on non-Linux platforms by default; on Linux only
# when GTK3 is detected. Override with --no-native-menus.
NATIVE_MENUS=auto

print_usage() {
    echo "Usage: ./build-gui.sh [OPTIONS]"
    echo ""
    echo "Builds the textamp GUI binary (Iced desktop app)."
    echo ""
    echo "Options:"
    echo "  --makepackage       Build and create platform-native package"
    echo "  --no-native-menus   Build without muda (no system GTK3 required)"
    echo "  --native-menus      Force muda native menus on"
    echo "  --clean             Remove build artifacts and packaging output"
    echo "  --help              Show this help message"
    echo ""
    echo "The GUI on Linux requires GTK3 when native menus are enabled."
    echo "  Arch:    sudo pacman -S gtk3"
    echo "  Debian:  sudo apt install libgtk-3-dev"
    echo "  Fedora:  sudo dnf install gtk3-devel"
}

detect_arch() {
    local machine
    machine=$(uname -m)
    case "$machine" in
        x86_64)          DEB_ARCH="amd64";  RPM_ARCH="x86_64";  ARCH_ARCH="x86_64" ;;
        aarch64|arm64)   DEB_ARCH="arm64";  RPM_ARCH="aarch64"; ARCH_ARCH="aarch64" ;;
        *)
            echo "Warning: Unknown architecture '$machine', defaulting to x86_64"
            DEB_ARCH="amd64"; RPM_ARCH="x86_64"; ARCH_ARCH="x86_64"
            ;;
    esac
}

detect_linux_distro() {
    if [ -f /etc/os-release ]; then
        . /etc/os-release
        DISTRO_ID="$ID"
        DISTRO_ID_LIKE="$ID_LIKE"
    else
        DISTRO_ID="unknown"
        DISTRO_ID_LIKE=""
    fi
}

feature_flags() {
    local feats=("gui")
    case "$NATIVE_MENUS" in
        yes) feats+=("native-menus") ;;
        auto)
            local os
            os=$(uname -s)
            if [ "$os" = "Darwin" ]; then
                feats+=("native-menus")
            elif [ "$os" = "Linux" ] && pkg-config --exists gdk-3.0 2>/dev/null; then
                feats+=("native-menus")
            elif [[ "$os" == MINGW* || "$os" == CYGWIN* || "$os" == MSYS* ]]; then
                feats+=("native-menus")
            fi
            ;;
    esac
    IFS=',' ; echo "${feats[*]}"
}

build_binary() {
    if ! command -v cargo &>/dev/null; then
        echo "Error: cargo is not installed."
        echo "Install Rust: https://rustup.rs"
        exit 1
    fi

    local features
    features="$(feature_flags)"

    echo "Building $PKG_NAME $PKG_VERSION GUI (release, features: $features)..."
    cargo build --release --no-default-features --features "$features" --bin textamp-gui

    echo ""
    echo "Build complete!"
    if [ -f "$BINARY_GUI" ]; then
        local full_path
        full_path="$(cd "$(dirname "$BINARY_GUI")" && pwd)/$(basename "$BINARY_GUI")"
        local size
        size=$(ls -lh "$BINARY_GUI" | awk '{print $5}')
        echo "  GUI: $full_path  ($size)"
    fi
}

do_clean() {
    echo "Cleaning build artifacts..."
    if [ -d target ]; then
        cargo clean
        echo "  Removed target/"
    fi
    if [ -d dist ]; then
        rm -rf dist
        echo "  Removed dist/"
    fi
    echo "Clean complete."
}

make_homebrew_formula() {
    echo ""
    echo "Creating Homebrew formula..."
    mkdir -p dist/homebrew
    local tarball="dist/homebrew/${PKG_NAME}-gui-${PKG_VERSION}.tar.gz"
    tar czf "$tarball" --exclude='target' --exclude='dist' --exclude='.git' --exclude='.DS_Store' .
    local tarball_path
    tarball_path="$(cd "$(dirname "$tarball")" && pwd)/$(basename "$tarball")"
    local sha256
    sha256=$(shasum -a 256 "$tarball" | awk '{print $1}')

    cat > dist/homebrew/${PKG_NAME}-gui.rb <<FORMULA
class TextampGui < Formula
  desc "$PKG_DESC"
  homepage "https://github.com/jbergmayer/textamp"
  url "file://$tarball_path"
  sha256 "$sha256"
  license "$PKG_LICENSE"
  version "$PKG_VERSION"

  depends_on "rust" => :build

  def install
    system "cargo", "build", "--release", "--no-default-features",
           "--features", "gui,native-menus", "--bin", "textamp-gui"
    bin.install "target/release/${PKG_NAME}-gui"
    doc.install "LICENSE"
    doc.install "config.example.toml"
  end
end
FORMULA

    echo "  Formula: dist/homebrew/${PKG_NAME}-gui.rb"
    echo "  Tarball: $tarball_path"
    echo ""
    echo "Install with:  brew install --formula dist/homebrew/${PKG_NAME}-gui.rb"
}

make_arch_package() {
    echo ""
    echo "Creating Arch Linux package..."
    local build_dir="dist/arch-gui"
    mkdir -p "$build_dir"
    tar czf "$build_dir/${PKG_NAME}-gui-${PKG_VERSION}.tar.gz" \
        --exclude='target' --exclude='dist' --exclude='.git' --exclude='.DS_Store' .

    local features
    features="$(feature_flags)"

    cat > "$build_dir/PKGBUILD" <<'PKGBUILD_HEADER'
# Maintainer: John Bergmayer
PKGBUILD_HEADER

    cat >> "$build_dir/PKGBUILD" <<PKGBUILD_BODY
pkgname=${PKG_NAME}-gui
pkgver=$PKG_VERSION
pkgrel=1
pkgdesc="$PKG_DESC"
arch=('x86_64' 'aarch64')
license=('$PKG_LICENSE')
depends=('openssl' 'alsa-lib' 'gtk3')
makedepends=('rust')
source=("${PKG_NAME}-gui-${PKG_VERSION}.tar.gz")
sha256sums=('SKIP')

build() {
    cd "\$srcdir"
    cargo build --release --no-default-features --features "$features" --bin textamp-gui
}

package() {
    cd "\$srcdir"
    install -Dm755 "target/release/${PKG_NAME}-gui" "\$pkgdir/usr/bin/${PKG_NAME}-gui"
    install -Dm644 "LICENSE" "\$pkgdir/usr/share/licenses/${PKG_NAME}-gui/LICENSE"
    install -Dm644 "config.example.toml" "\$pkgdir/usr/share/doc/${PKG_NAME}-gui/config.example.toml"
}
PKGBUILD_BODY

    echo "  PKGBUILD: $build_dir/PKGBUILD"
    if command -v makepkg &>/dev/null; then
        (cd "$build_dir" && makepkg -f)
        local pkg_file
        pkg_file=$(ls "$build_dir"/${PKG_NAME}-gui-${PKG_VERSION}-*.pkg.tar.zst 2>/dev/null | head -1)
        [ -n "$pkg_file" ] && echo -e "\nInstall with:  sudo pacman -U $pkg_file"
    else
        echo "  makepkg not found — PKGBUILD generated but package not built"
    fi
}

make_deb_package() {
    echo ""
    echo "Creating Debian package..."
    detect_arch
    local pkg_dir="dist/deb-gui/${PKG_NAME}-gui_${PKG_VERSION}_${DEB_ARCH}"
    mkdir -p "$pkg_dir/DEBIAN" "$pkg_dir/usr/bin" "$pkg_dir/usr/share/doc/${PKG_NAME}-gui"

    cat > "$pkg_dir/DEBIAN/control" <<CONTROL
Package: ${PKG_NAME}-gui
Version: $PKG_VERSION
Architecture: $DEB_ARCH
Maintainer: $PKG_MAINTAINER
Description: $PKG_DESC
Depends: libssl3 | libssl1.1, libasound2, libgtk-3-0
Section: sound
Priority: optional
CONTROL

    cp "$BINARY_GUI" "$pkg_dir/usr/bin/${PKG_NAME}-gui"
    chmod 755 "$pkg_dir/usr/bin/${PKG_NAME}-gui"
    cp LICENSE "$pkg_dir/usr/share/doc/${PKG_NAME}-gui/"
    cp config.example.toml "$pkg_dir/usr/share/doc/${PKG_NAME}-gui/"

    if command -v dpkg-deb &>/dev/null; then
        local deb_file="dist/${PKG_NAME}-gui_${PKG_VERSION}_${DEB_ARCH}.deb"
        dpkg-deb --build "$pkg_dir" "$deb_file"
        echo "  Package: $deb_file"
        echo -e "\nInstall with:  sudo dpkg -i $deb_file"
    else
        echo "  dpkg-deb not found — package directory prepared at $pkg_dir"
    fi
}

make_rpm_package() {
    echo ""
    echo "Creating RPM package..."
    detect_arch
    local rpm_topdir="dist/rpm-gui"
    mkdir -p "$rpm_topdir"/{BUILD,RPMS,SOURCES,SPECS,SRPMS,BUILDROOT}
    cp "$BINARY_GUI" "$rpm_topdir/SOURCES/${PKG_NAME}-gui"
    cp LICENSE config.example.toml "$rpm_topdir/SOURCES/"

    cat > "$rpm_topdir/SPECS/${PKG_NAME}-gui.spec" <<SPEC
Name:           ${PKG_NAME}-gui
Version:        $PKG_VERSION
Release:        1%{?dist}
Summary:        $PKG_DESC
License:        $PKG_LICENSE
URL:            https://github.com/jbergmayer/textamp
Requires:       openssl-libs, alsa-lib, gtk3

%description
$PKG_DESC

%install
mkdir -p %{buildroot}/usr/bin
mkdir -p %{buildroot}/usr/share/doc/${PKG_NAME}-gui
install -m 755 %{_sourcedir}/${PKG_NAME}-gui %{buildroot}/usr/bin/${PKG_NAME}-gui
install -m 644 %{_sourcedir}/LICENSE %{buildroot}/usr/share/doc/${PKG_NAME}-gui/LICENSE
install -m 644 %{_sourcedir}/config.example.toml %{buildroot}/usr/share/doc/${PKG_NAME}-gui/config.example.toml

%files
/usr/bin/${PKG_NAME}-gui
/usr/share/doc/${PKG_NAME}-gui/LICENSE
/usr/share/doc/${PKG_NAME}-gui/config.example.toml
SPEC

    if command -v rpmbuild &>/dev/null; then
        rpmbuild --define "_topdir $(pwd)/$rpm_topdir" -bb "$rpm_topdir/SPECS/${PKG_NAME}-gui.spec"
        local rpm_file
        rpm_file=$(ls "$rpm_topdir"/RPMS/${RPM_ARCH}/${PKG_NAME}-gui-*.rpm 2>/dev/null | head -1)
        [ -n "$rpm_file" ] && echo -e "\nInstall with:  sudo rpm -i $rpm_file"
    else
        echo "  rpmbuild not found — spec file generated but package not built"
    fi
}

make_package() {
    local os
    os=$(uname -s)
    case "$os" in
        Darwin) make_homebrew_formula ;;
        Linux)
            detect_linux_distro
            case "$DISTRO_ID" in
                arch|manjaro|endeavouros) make_arch_package ;;
                debian|ubuntu|pop|linuxmint|elementary|zorin) make_deb_package ;;
                fedora|rhel|centos|rocky|alma|ol) make_rpm_package ;;
                *)
                    if echo "$DISTRO_ID_LIKE" | grep -q "arch"; then
                        make_arch_package
                    elif echo "$DISTRO_ID_LIKE" | grep -q "debian\|ubuntu"; then
                        make_deb_package
                    elif echo "$DISTRO_ID_LIKE" | grep -q "fedora\|rhel"; then
                        make_rpm_package
                    else
                        echo -e "\nUnknown Linux distribution: $DISTRO_ID"
                        echo "Binary built successfully. Install manually:"
                        echo "  sudo cp $BINARY_GUI /usr/local/bin/${PKG_NAME}-gui"
                    fi
                    ;;
            esac
            ;;
        *) echo "Error: Unsupported OS '$os'. Only macOS and Linux are supported."; exit 1 ;;
    esac
}

MODE=build
while [ $# -gt 0 ]; do
    case "$1" in
        --help|-h)            print_usage; exit 0 ;;
        --clean)              MODE=clean ;;
        --makepackage)        MODE=package ;;
        --no-native-menus)    NATIVE_MENUS=no ;;
        --native-menus)       NATIVE_MENUS=yes ;;
        *) echo "Error: Unknown option '$1'"; print_usage; exit 1 ;;
    esac
    shift
done

case "$MODE" in
    build)   build_binary ;;
    clean)   do_clean ;;
    package) build_binary; make_package ;;
esac
