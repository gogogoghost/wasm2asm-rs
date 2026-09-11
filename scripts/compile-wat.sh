#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

cargo build --quiet --example compile_wat
compiler="$root/target/debug/examples/compile_wat"

if (($#)); then
  exec "$compiler" "$@"
fi

fixture_root="$root/tests/fixtures"
output_root="$root/target/wasm-fixtures"
while IFS= read -r -d '' source; do
  relative=${source#"$fixture_root"/}
  output="$output_root/${relative%.wat}.wasm"
  "$compiler" "$source" "$output"
done < <(find "$fixture_root" -type f -name '*.wat' -print0 | sort -z)
