#!/bin/sh
set -e

# Tags are pushed as v1.3.870; the archive name carries the bare version.
VERSION="${DRONE_TAG#v}"

if [ -z "$VERSION" ]; then
  echo "DRONE_TAG is empty, cannot derive a release version"
  exit 1
fi

mkdir -p /release

pack_zip() {
  DIR="$1"
  SUFFIX="$2"
  BIN="$3"
  NAME="open-football_${VERSION}_${SUFFIX}.zip"
  echo "Packing ${NAME}"
  (cd "/artifacts/${DIR}" && zip -q -9 "/release/${NAME}" "$BIN")
}

pack_tar() {
  DIR="$1"
  SUFFIX="$2"
  BIN="$3"
  NAME="open-football_${VERSION}_${SUFFIX}.tar.gz"
  echo "Packing ${NAME}"
  chmod +x "/artifacts/${DIR}/${BIN}"
  (cd "/artifacts/${DIR}" && tar czf "/release/${NAME}" "$BIN")
}

pack_zip windows      windows      open_football.exe
pack_tar linux        linux        open_football
pack_tar mac_intel    mac_intel    open_football
pack_tar mac_m_series mac_m_series open_football

echo "Packaged artifacts:"
ls -la /release
