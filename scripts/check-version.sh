#!/bin/sh
# 릴리스 버전이 네 곳에서 같은지 확인한다. 사용법: scripts/check-version.sh <버전>   (예: 0.1.0)
# release CI의 첫 단계이며, 로컬에서도 같은 명령으로 확인할 수 있다.
set -eu
want=${1:?usage: check-version.sh <version>}
cd "$(dirname "$0")/.."

cargo=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
plugin=$(sed -n 's/^ *"version": "\(.*\)",*$/\1/p' .claude-plugin/plugin.json | head -1)
sh_v=$(sed -n 's/^VERSION=\([^ ]*\).*$/\1/p' skills/pdf2md/scripts/install.sh | head -1)
ps_v=$(sed -n "s/^\$Version = '\(.*\)'.*\$/\1/p" skills/pdf2md/scripts/install.ps1 | head -1)

bad=0
for pair in "Cargo.toml:$cargo" "plugin.json:$plugin" "install.sh:$sh_v" "install.ps1:$ps_v"; do
  name=${pair%%:*}; got=${pair#*:}
  if [ "$got" != "$want" ]; then
    echo "version mismatch: $name has '$got', expected '$want'" >&2
    bad=1
  fi
done
[ $bad -eq 0 ] && echo "all versions are $want"
exit $bad
