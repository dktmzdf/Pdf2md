#!/bin/sh
# pdf2md 바이너리와 OCR 모델을 <data-dir>/bin/ 아래에 설치한다(Linux x86_64 전용).
# 사용법: install.sh <data-dir>      (skill은 <data-dir>로 ${CLAUDE_PLUGIN_DATA}를 넘긴다)
# - 이미 같은 버전이 설치되어 있고 모델 3개가 다 있으면 아무것도 받지 않는다(네트워크 없음).
# - 바이너리 압축은 같은 Release의 SHA256SUMS로, 모델은 아래 고정 해시로 검증한다.
# - 검증을 통과한 파일만 제자리로 옮기므로, 실패해도 기존 정상 파일은 그대로 남는다.
set -eu

VERSION=0.1.0 # Cargo.toml, .claude-plugin/plugin.json, install.ps1과 같아야 한다(release CI가 확인)
REPO=dktmzdf/Pdf2md
# 테스트에서 로컬 서버로 바꿔 끼우기 위한 값. 평소에는 실제 주소를 쓴다.
RELEASE_BASE=${PDF2MD_RELEASE_BASE:-https://github.com/$REPO/releases/download/v$VERSION}
HF_BASE=${PDF2MD_HF_BASE:-https://huggingface.co}
OS=${PDF2MD_OS:-$(uname -s)}
ARCH=${PDF2MD_ARCH:-$(uname -m)}

DET_COMMIT=e7bd63d9c674c96bd4fdab435af0efb383919c18
REC_COMMIT=5c6f574b8e2230adf4287b33e736d71b9fabd28e

die() { echo "pdf2md install: $*" >&2; exit 1; }

[ $# -eq 1 ] || die "usage: install.sh <data-dir>"
case "$OS-$ARCH" in
  Linux-x86_64) TARGET=x86_64-unknown-linux-gnu ;;
  *) die "unsupported platform: $OS $ARCH (supported: Linux x86_64, Windows x86_64)" ;;
esac
command -v curl >/dev/null || die "curl is required"
command -v sha256sum >/dev/null || die "sha256sum is required"

BIN=$1/bin
MODELS=$BIN/models
mkdir -p "$MODELS"
TMP=$(mktemp -d "$1/.install.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

fetch() { # url out
  curl -fsSL --retry 2 -o "$2" "$1" || die "download failed: $1"
}
verify() { # file expected-sha256
  actual=$(sha256sum "$1" | cut -d' ' -f1)
  [ "$actual" = "$2" ] || die "checksum mismatch for $(basename "$1"): expected $2, got $actual"
}

# 바이너리: 버전이 다르거나 없을 때만 받는다.
if [ "$("$BIN/pdf2md" --version 2>/dev/null || true)" != "pdf2md $VERSION" ]; then
  ARCHIVE=pdf2md-$TARGET.tar.gz
  fetch "$RELEASE_BASE/SHA256SUMS" "$TMP/SHA256SUMS"
  fetch "$RELEASE_BASE/$ARCHIVE" "$TMP/$ARCHIVE"
  expected=$(awk -v f="$ARCHIVE" '$2 == f || $2 == "*" f { print $1 }' "$TMP/SHA256SUMS")
  [ -n "$expected" ] || die "$ARCHIVE is not listed in SHA256SUMS"
  verify "$TMP/$ARCHIVE" "$expected"
  mkdir "$TMP/x"
  tar -xzf "$TMP/$ARCHIVE" -C "$TMP/x"
  [ -f "$TMP/x/pdf2md" ] || die "$ARCHIVE does not contain pdf2md"
  chmod +x "$TMP/x/pdf2md"
  mv -f "$TMP/x/pdf2md" "$BIN/pdf2md"
fi

# 모델: 없는 파일만 받는다. 검증 후 옮기므로 있는 파일은 이미 검증된 것이다.
model() { # name url sha256
  [ -f "$MODELS/$1" ] && return 0
  fetch "$2" "$TMP/$1"
  verify "$TMP/$1" "$3"
  mv -f "$TMP/$1" "$MODELS/$1"
}
model det.onnx "$HF_BASE/deepghs/paddleocr/resolve/$DET_COMMIT/det/ch_PP-OCRv4_det/model.onnx" \
  30a86f5731181461d08021402766601e4302a9b9b9666be8aff402696339cdff
model rec.onnx "$HF_BASE/PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx/resolve/$REC_COMMIT/inference.onnx" \
  92f0b7785e64fc9090106a241cf4c1eb97472824558272751b88a2a4476d3a08
model inference.yml "$HF_BASE/PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx/resolve/$REC_COMMIT/inference.yml" \
  f757fa1c40e99edcf27e9cce879b93eb2a51fa46f5ef39095689b8c37dd75998

echo "pdf2md $VERSION ready: $BIN/pdf2md"
