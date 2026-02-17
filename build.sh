#!/bin/bash
# Build script for textamp - works on macOS and Linux
# Usage: ./build.sh [--makepackage | --clean | --help]

set -e

# ── Project metadata (extracted from Cargo.toml) ─────────────────────────

PKG_NAME="textamp"
PKG_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
PKG_DESC="A keyboard-driven TUI client for Plex Music"
PKG_LICENSE="MIT"
PKG_AUTHOR="John Bergmayer"
PKG_MAINTAINER="John Bergmayer"
BINARY="target/release/$PKG_NAME"

# ── Helpers ───────────────────────────────────────────────────────────────

print_usage() {
    echo "Usage: ./build.sh [OPTION]"
    echo ""
    echo "Options:"
    echo "  (none)          Build release binary"
    echo "  --makepackage   Build binary and create platform package"
    echo "  --clean         Clean build artifacts and packaging output"
    echo "  --help          Show this help message"
}

detect_arch() {
    local machine
    machine=$(uname -m)
    case "$machine" in
        x86_64)
            DEB_ARCH="amd64"
            RPM_ARCH="x86_64"
            ARCH_ARCH="x86_64"
            ;;
        aarch64|arm64)
            DEB_ARCH="arm64"
            RPM_ARCH="aarch64"
            ARCH_ARCH="aarch64"
            ;;
        *)
            echo "Warning: Unknown architecture '$machine', defaulting to x86_64"
            DEB_ARCH="amd64"
            RPM_ARCH="x86_64"
            ARCH_ARCH="x86_64"
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

# ── Build ─────────────────────────────────────────────────────────────────

build_binary() {
    if ! command -v cargo &>/dev/null; then
        echo "Error: cargo is not installed."
        echo "Install Rust: https://rustup.rs"
        exit 1
    fi

    echo "Building $PKG_NAME $PKG_VERSION (release)..."
    cargo build --release

    if [ ! -f "$BINARY" ]; then
        echo "Error: Binary not found at $BINARY"
        exit 1
    fi

    FULL_PATH="$(cd "$(dirname "$BINARY")" && pwd)/$(basename "$BINARY")"
    SIZE=$(ls -lh "$BINARY" | awk '{print $5}')

    echo ""
    echo "Build complete!"
    echo "  Binary:  $FULL_PATH"
    echo "  Size:    $SIZE"
}

# ── Clean ─────────────────────────────────────────────────────────────────

do_clean() {
    echo "Cleaning build artifacts..."

    if [ -d target ]; then
        cargo clean
        echo "  Removed target/"
    else
        echo "  target/ not found, skipping"
    fi

    if [ -d dist ]; then
        rm -rf dist
        echo "  Removed dist/"
    else
        echo "  dist/ not found, skipping"
    fi

    echo "Clean complete."
}

# ── Packaging: macOS Homebrew ─────────────────────────────────────────────

make_homebrew_formula() {
    echo ""
    echo "Creating Homebrew formula..."

    mkdir -p dist/homebrew

    # Create a source tarball for the formula
    local tarball="dist/homebrew/${PKG_NAME}-${PKG_VERSION}.tar.gz"
    tar czf "$tarball" \
        --exclude='target' \
        --exclude='dist' \
        --exclude='.git' \
        --exclude='.DS_Store' \
        .

    local tarball_path
    tarball_path="$(cd "$(dirname "$tarball")" && pwd)/$(basename "$tarball")"
    local sha256
    sha256=$(shasum -a 256 "$tarball" | awk '{print $1}')

    cat > dist/homebrew/${PKG_NAME}.rb <<FORMULA
class Textamp < Formula
  desc "$PKG_DESC"
  homepage "https://github.com/jbergmayer/textamp"
  url "file://$tarball_path"
  sha256 "$sha256"
  license "$PKG_LICENSE"
  version "$PKG_VERSION"

  depends_on "rust" => :build
  depends_on "chafa"
  depends_on "glib"
  depends_on "gettext"

  def install
    system "cargo", "build", "--release"
    bin.install "target/release/$PKG_NAME"
    doc.install "LICENSE"
    doc.install "config.example.toml"
  end

  test do
    assert_match "$PKG_VERSION", shell_output("#{bin}/$PKG_NAME --version 2>&1", 1)
  end
end
FORMULA

    echo "  Formula: dist/homebrew/${PKG_NAME}.rb"
    echo "  Tarball: $tarball_path"
    echo ""
    echo "Install with:"
    echo "  brew install --formula dist/homebrew/${PKG_NAME}.rb"
}

# ── Packaging: Arch Linux PKGBUILD ────────────────────────────────────────

make_arch_package() {
    echo ""
    echo "Creating Arch Linux package..."

    local build_dir="dist/arch"
    mkdir -p "$build_dir"

    # Copy source into build dir
    local src_dir="$build_dir/src"
    mkdir -p "$src_dir"
    tar czf "$build_dir/${PKG_NAME}-${PKG_VERSION}.tar.gz" \
        --exclude='target' \
        --exclude='dist' \
        --exclude='.git' \
        --exclude='.DS_Store' \
        .

    cat > "$build_dir/PKGBUILD" <<'PKGBUILD_HEADER'
# Maintainer: John Bergmayer
PKGBUILD_HEADER

    cat >> "$build_dir/PKGBUILD" <<PKGBUILD_BODY
pkgname=$PKG_NAME
pkgver=$PKG_VERSION
pkgrel=1
pkgdesc="$PKG_DESC"
arch=('x86_64' 'aarch64')
license=('$PKG_LICENSE')
depends=('openssl' 'alsa-lib' 'chafa' 'glib2')
makedepends=('rust')
source=("${PKG_NAME}-${PKG_VERSION}.tar.gz")
sha256sums=('SKIP')

build() {
    cd "\$srcdir"
    cargo build --release
}

package() {
    cd "\$srcdir"
    install -Dm755 "target/release/$PKG_NAME" "\$pkgdir/usr/bin/$PKG_NAME"
    install -Dm644 "LICENSE" "\$pkgdir/usr/share/licenses/$PKG_NAME/LICENSE"
    install -Dm644 "config.example.toml" "\$pkgdir/usr/share/doc/$PKG_NAME/config.example.toml"
}
PKGBUILD_BODY

    echo "  PKGBUILD: $build_dir/PKGBUILD"

    if command -v makepkg &>/dev/null; then
        echo "  Running makepkg..."
        (cd "$build_dir" && makepkg -f)
        local pkg_file
        pkg_file=$(ls "$build_dir"/${PKG_NAME}-${PKG_VERSION}-*.pkg.tar.zst 2>/dev/null | head -1)
        if [ -n "$pkg_file" ]; then
            echo ""
            echo "Install with:"
            echo "  sudo pacman -U $pkg_file"
        fi
    else
        echo "  makepkg not found — PKGBUILD generated but package not built"
        echo "  Run manually: cd $build_dir && makepkg -f"
    fi
}

# ── Packaging: Debian/Ubuntu .deb ─────────────────────────────────────────

make_deb_package() {
    echo ""
    echo "Creating Debian package..."

    detect_arch

    local pkg_dir="dist/deb/${PKG_NAME}_${PKG_VERSION}_${DEB_ARCH}"
    mkdir -p "$pkg_dir/DEBIAN"
    mkdir -p "$pkg_dir/usr/bin"
    mkdir -p "$pkg_dir/usr/share/doc/$PKG_NAME"

    # Control file
    cat > "$pkg_dir/DEBIAN/control" <<CONTROL
Package: $PKG_NAME
Version: $PKG_VERSION
Architecture: $DEB_ARCH
Maintainer: $PKG_MAINTAINER
Description: $PKG_DESC
Depends: libssl3 | libssl1.1, libasound2, chafa, libglib2.0-0
Section: sound
Priority: optional
CONTROL

    # Install files
    cp "$BINARY" "$pkg_dir/usr/bin/$PKG_NAME"
    chmod 755 "$pkg_dir/usr/bin/$PKG_NAME"
    cp LICENSE "$pkg_dir/usr/share/doc/$PKG_NAME/"
    cp config.example.toml "$pkg_dir/usr/share/doc/$PKG_NAME/"

    # Build .deb
    if command -v dpkg-deb &>/dev/null; then
        local deb_file="dist/${PKG_NAME}_${PKG_VERSION}_${DEB_ARCH}.deb"
        dpkg-deb --build "$pkg_dir" "$deb_file"
        echo "  Package: $deb_file"
        echo ""
        echo "Install with:"
        echo "  sudo dpkg -i $deb_file"
    else
        echo "  dpkg-deb not found — package directory prepared at $pkg_dir"
        echo "  Install dpkg-deb or copy the binary manually"
    fi
}

# ── Packaging: Fedora/RHEL .rpm ───────────────────────────────────────────

make_rpm_package() {
    echo ""
    echo "Creating RPM package..."

    detect_arch

    local rpm_topdir="dist/rpm"
    mkdir -p "$rpm_topdir"/{BUILD,RPMS,SOURCES,SPECS,SRPMS,BUILDROOT}

    # Copy binary into SOURCES
    cp "$BINARY" "$rpm_topdir/SOURCES/$PKG_NAME"
    cp LICENSE "$rpm_topdir/SOURCES/"
    cp config.example.toml "$rpm_topdir/SOURCES/"

    # Spec file (installs pre-built binary, no rebuild)
    cat > "$rpm_topdir/SPECS/${PKG_NAME}.spec" <<SPEC
Name:           $PKG_NAME
Version:        $PKG_VERSION
Release:        1%{?dist}
Summary:        $PKG_DESC
License:        $PKG_LICENSE
URL:            https://github.com/jbergmayer/textamp

Requires:       openssl-libs, alsa-lib, chafa, glib2

%description
$PKG_DESC

%install
mkdir -p %{buildroot}/usr/bin
mkdir -p %{buildroot}/usr/share/doc/$PKG_NAME
install -m 755 %{_sourcedir}/$PKG_NAME %{buildroot}/usr/bin/$PKG_NAME
install -m 644 %{_sourcedir}/LICENSE %{buildroot}/usr/share/doc/$PKG_NAME/LICENSE
install -m 644 %{_sourcedir}/config.example.toml %{buildroot}/usr/share/doc/$PKG_NAME/config.example.toml

%files
/usr/bin/$PKG_NAME
/usr/share/doc/$PKG_NAME/LICENSE
/usr/share/doc/$PKG_NAME/config.example.toml
SPEC

    echo "  Spec: $rpm_topdir/SPECS/${PKG_NAME}.spec"

    if command -v rpmbuild &>/dev/null; then
        echo "  Running rpmbuild..."
        rpmbuild --define "_topdir $(pwd)/$rpm_topdir" -bb "$rpm_topdir/SPECS/${PKG_NAME}.spec"
        local rpm_file
        rpm_file=$(ls "$rpm_topdir"/RPMS/${RPM_ARCH}/${PKG_NAME}-*.rpm 2>/dev/null | head -1)
        if [ -n "$rpm_file" ]; then
            echo "  Package: $rpm_file"
            echo ""
            echo "Install with:"
            echo "  sudo rpm -i $rpm_file"
        fi
    else
        echo "  rpmbuild not found — spec file generated but package not built"
        echo "  Install rpm-build and run: rpmbuild --define '_topdir $(pwd)/$rpm_topdir' -bb $rpm_topdir/SPECS/${PKG_NAME}.spec"
    fi
}

# ── Package dispatch ──────────────────────────────────────────────────────

make_package() {
    local os
    os=$(uname -s)

    case "$os" in
        Darwin)
            make_homebrew_formula
            ;;
        Linux)
            detect_linux_distro
            case "$DISTRO_ID" in
                arch|manjaro|endeavouros)
                    make_arch_package
                    ;;
                debian|ubuntu|pop|linuxmint|elementary|zorin)
                    make_deb_package
                    ;;
                fedora|rhel|centos|rocky|alma|ol)
                    make_rpm_package
                    ;;
                *)
                    # Check ID_LIKE for derivative distros
                    if echo "$DISTRO_ID_LIKE" | grep -q "arch"; then
                        make_arch_package
                    elif echo "$DISTRO_ID_LIKE" | grep -q "debian\|ubuntu"; then
                        make_deb_package
                    elif echo "$DISTRO_ID_LIKE" | grep -q "fedora\|rhel"; then
                        make_rpm_package
                    else
                        echo ""
                        echo "Unknown Linux distribution: $DISTRO_ID"
                        echo "Binary built successfully. Install manually:"
                        echo "  sudo cp $BINARY /usr/local/bin/$PKG_NAME"
                    fi
                    ;;
            esac
            ;;
        *)
            echo "Error: Unsupported OS '$os'. Only macOS and Linux are supported."
            exit 1
            ;;
    esac
}

# ── Main ──────────────────────────────────────────────────────────────────

case "${1:-}" in
    --help|-h)
        print_usage
        ;;
    --clean)
        do_clean
        ;;
    --makepackage)
        build_binary
        make_package
        ;;
    "")
        build_binary
        ;;
    *)
        echo "Error: Unknown option '$1'"
        print_usage
        exit 1
        ;;
esac
