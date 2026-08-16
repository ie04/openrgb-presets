# Maintainer: ie04 <iyadeltifi12@gmail.com>

pkgname=openrgb-presets-git
_pkgname=${pkgname%-git}
pkgver=0.1.0.r4.g8359bce
pkgrel=1
pkgdesc='OpenRGB background presets and keypress ripple effects for Logitech G513 and G502 HERO devices'
arch=('x86_64')
url='https://github.com/ie04/openrgb-presets'
license=('LicenseRef-All-rights-reserved')
depends=('gcc-libs' 'glibc' 'openrgb')
makedepends=('cargo' 'git')
provides=("${_pkgname}")
conflicts=("${_pkgname}")
source=("${_pkgname}::git+${url}.git")
b2sums=('SKIP')

pkgver() {
  cd "${_pkgname}"

  local version
  version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml)
  printf '%s.r%s.g%s' \
    "${version}" \
    "$(git rev-list --count HEAD)" \
    "$(git rev-parse --short=7 HEAD)"
}

prepare() {
  cd "${_pkgname}"
  CARGO_HOME="${srcdir}/cargo-home" cargo fetch --locked --target x86_64-unknown-linux-gnu
}

build() {
  cd "${_pkgname}"
  export RUSTFLAGS="${RUSTFLAGS} --remap-path-prefix=${srcdir}=/"
  CARGO_HOME="${srcdir}/cargo-home" \
    CARGO_TARGET_DIR=target \
    cargo build --frozen --release
}

check() {
  cd "${_pkgname}"
  CARGO_HOME="${srcdir}/cargo-home" \
    CARGO_TARGET_DIR=target \
    cargo test --frozen
}

package() {
  cd "${_pkgname}"
  install -Dm755 target/release/openrgb-presets \
    "${pkgdir}/usr/bin/openrgb-presets"
  install -Dm644 README.md \
    "${pkgdir}/usr/share/doc/${pkgname}/README.md"
  install -Dm644 openrgb-presets.service \
    "${pkgdir}/usr/lib/systemd/user/openrgb-presets.service"
  install -Dm644 LICENSE \
    "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
}
