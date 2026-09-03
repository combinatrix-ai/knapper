#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd -- "$script_dir/../.." && pwd -P)"

if (( $# > 1 )); then
  printf 'usage: %s [output.zip]\n' "$0" >&2
  exit 2
fi

output_path="${1:-$repo_root/target/knapper-chrome-extension.zip}"
if [[ "$output_path" != /* ]]; then
  output_path="$repo_root/$output_path"
fi
case "$output_path" in
  *.zip) ;;
  *)
    printf 'error: output path must end in .zip: %s\n' "$output_path" >&2
    exit 2
    ;;
esac

if ! command -v zip >/dev/null 2>&1; then
  printf 'error: zip is required to package the Chrome extension\n' >&2
  exit 1
fi
if ! command -v unzip >/dev/null 2>&1; then
  printf 'error: unzip is required to verify the Chrome extension archive\n' >&2
  exit 1
fi

runtime_files=(
  manifest.json
  service_worker.js
  content_script.js
  popup.html
  popup.js
  popup.css
)

mkdir -p "$(dirname -- "$output_path")"
staging_dir="$(mktemp -d "${TMPDIR:-/tmp}/knapper-chrome-extension.XXXXXX")"
cleanup() {
  rm -rf "$staging_dir"
}
trap cleanup EXIT

for file in "${runtime_files[@]}"; do
  source_path="$script_dir/$file"
  if [[ ! -f "$source_path" ]]; then
    printf 'error: required extension file is missing: %s\n' "$source_path" >&2
    exit 1
  fi
  cp "$source_path" "$staging_dir/$file"
done

rm -f "$output_path"
(
  cd "$staging_dir"
  zip -q -X "$output_path" "${runtime_files[@]}"
)

unzip -tqq "$output_path"
expected_list="$staging_dir/expected.list"
actual_list="$staging_dir/actual.list"
printf '%s\n' "${runtime_files[@]}" | LC_ALL=C sort >"$expected_list"
LC_ALL=C unzip -Z1 "$output_path" | LC_ALL=C sort >"$actual_list"
if ! cmp -s "$expected_list" "$actual_list"; then
  printf 'error: archive contents differ from the runtime extension file set\n' >&2
  printf '%s\n' 'expected:' >&2
  sed 's/^/  /' "$expected_list" >&2
  printf '%s\n' 'actual:' >&2
  sed 's/^/  /' "$actual_list" >&2
  exit 1
fi

printf 'created Chrome extension archive: %s\n' "$output_path"
