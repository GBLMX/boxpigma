# Maintainer: GB LMX <GBLMX@users.noreply.github.com>
pkgname=boxpigma-gblmx-bin
_pkgname=boxpigma
pkgver=1.6.3
pkgrel=1
pkgdesc="A netease cloud music client (GBLMX fork build)"
arch=('x86_64')
url="https://github.com/GBLMX/pigma"
license=('Apache-2.0')
# The binary is installed as /usr/bin/boxpigma (upstream's name), so this declares what it
# really provides and conflicts with anything else that owns it. Note that upstream has no AUR
# package: `boxpigma-bin` does not exist there.
provides=("${_pkgname}")
conflicts=("${_pkgname}")
source=("${_pkgname}-${pkgver}.tar.gz::${url}/releases/download/v${pkgver}/${_pkgname}-x86_64-unknown-linux-gnu.tar.gz")
# Checksum of the release asset referenced above. Nothing computes this automatically: bump it
# together with `pkgver` (`curl -sSL <the asset> | sha256sum`), or `makepkg` will refuse.
sha256sums=('4bb198271edc7d96aad0438298d94887ac656037fa18b7b9fabec5b2f81c1d49')

package() {
    install -Dm755 "${srcdir}/${_pkgname}" "${pkgdir}/usr/bin/${_pkgname}"
    # Newer archives carry the iced desktop frontend; v1.6.3 and older do not.
    if [ -f "${srcdir}/${_pkgname}-gui" ]; then
        install -Dm755 "${srcdir}/${_pkgname}-gui" "${pkgdir}/usr/bin/${_pkgname}-gui"
    fi

    # install -Dm644 "${srcdir}/LICENSE" "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
}
