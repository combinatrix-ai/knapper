#!/bin/sh
# Install knapper, and register its agent skill.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/combinatrix-ai/knapper/main/install.sh | sh
#   sh install.sh --version v0.1.0 --skill claude
#   sh install.sh --register-skills-from ./target/release/knapper --skill both

set -eu

KNAPPER_GITHUB_REPO="combinatrix-ai/knapper"
KNAPPER_BINARY_NAME="knapper"
KNAPPER_INSTALLER_NO_MAIN="${KNAPPER_INSTALLER_NO_MAIN:-0}"

die() {
  printf 'knapper installer: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: install.sh [options]

Install the published knapper binary into a user-writable directory.

Options:
  --version VERSION  Install VERSION (for example v0.1.0); default: latest
  --bin-dir DIR      Install into DIR; default: $KNAPPER_BIN_DIR or ~/.local/bin
  --skill MODE       Register the embedded skill: auto, none, codex, claude, or both
  --register-skills-from BINARY
                     Register skills from an existing knapper binary; skip download
  --no-skill         Alias for --skill none
  -h, --help         Show this help
EOF
}

need_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

# Linux builds are musl-linked and static, so one artefact serves glibc and
# musl systems alike; there is nothing to detect beyond the architecture.
detect_target() {
  target_os="$1"
  target_arch="$2"

  case "$target_arch" in
    x86_64|amd64) arch_target="x86_64" ;;
    arm64|aarch64) arch_target="aarch64" ;;
    *)
      printf 'unsupported architecture: %s (supported: x86_64 and arm64)\n' "$target_arch" >&2
      return 1
      ;;
  esac

  case "$target_os" in
    Darwin) printf '%s-apple-darwin\n' "$arch_target" ;;
    Linux) printf '%s-unknown-linux-musl\n' "$arch_target" ;;
    *)
      printf 'unsupported operating system: %s (supported: macOS and Linux)\n' "$target_os" >&2
      return 1
      ;;
  esac
}

release_asset_name() {
  printf '%s-%s-%s.tar.gz\n' "$KNAPPER_BINARY_NAME" "$1" "$2"
}

validate_version() {
  candidate="$1"
  case "$candidate" in
    *[!A-Za-z0-9.+_-]*) die "invalid version '$candidate'" ;;
  esac
  normalized="${candidate#v}"
  core="${normalized%%[-+]*}"
  old_ifs="$IFS"
  IFS=.
  set -- $core
  IFS="$old_ifs"
  [ "$#" -eq 3 ] || die "invalid version '$candidate'; expected v1.2.3 or 1.2.3"
  for component in "$@"; do
    case "$component" in
      ''|*[!0-9]*) die "invalid version '$candidate'; expected v1.2.3 or 1.2.3" ;;
    esac
  done
}

resolve_release_tag() {
  requested="$1"
  if [ "$requested" = "latest" ]; then
    latest_url="https://github.com/${KNAPPER_GITHUB_REPO}/releases/latest"
    final_url="$(curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
      --output /dev/null --write-out '%{url_effective}' "$latest_url")" \
      || die "could not resolve the latest knapper release"
    release_tag="${final_url##*/}"
  else
    case "$requested" in
      v*) release_tag="$requested" ;;
      *) release_tag="v$requested" ;;
    esac
  fi
  validate_version "$release_tag"
  printf '%s\n' "$release_tag"
}

download() {
  curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
    --output "$2" "$1" \
    || die "download failed: $1"
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    die "required checksum command not found: sha256sum or shasum"
  fi
}

# A release publishes one SHA256SUMS covering every artefact, so the digest
# has to be selected by filename rather than read off the top.
verify_checksum() {
  archive_path="$1"
  sums_path="$2"
  archive_file="$(basename "$archive_path")"
  expected="$(awk -v want="$archive_file" '
    { name = $2; sub(/^\*/, "", name); if (name == want) { print $1; exit } }
  ' "$sums_path")"
  case "$expected" in
    "") die "SHA256SUMS has no entry for $archive_file" ;;
    *[!0-9A-Fa-f]*) die "SHA256SUMS holds an invalid digest for $archive_file" ;;
  esac
  actual="$(sha256 "$archive_path")"
  expected="$(printf '%s' "$expected" | tr 'A-F' 'a-f')"
  [ "$actual" = "$expected" ] || die "checksum verification failed for $archive_file"
}

install_skill() {
  skill_target="$1"
  skill_directory="$(dirname "$skill_target")"
  mkdir -p "$skill_directory" || die "could not create skill directory: $skill_directory"
  skill_temp="$(mktemp "$skill_directory/.knapper-skill.XXXXXX")" \
    || die "could not create a temporary skill file in $skill_directory"
  if ! "$installed_path" skill > "$skill_temp"; then
    rm -f "$skill_temp"
    die "could not read the embedded knapper skill"
  fi

  if [ -f "$skill_target" ] && cmp -s "$skill_temp" "$skill_target"; then
    rm -f "$skill_temp"
    printf 'knapper skill already current: %s\n' "$skill_target"
    return 0
  fi

  # Never discard a skill somebody may have edited.
  if [ -e "$skill_target" ] || [ -L "$skill_target" ]; then
    skill_backup="${skill_target}.backup.$(date +%Y%m%d%H%M%S)"
    mv "$skill_target" "$skill_backup" \
      || { rm -f "$skill_temp"; die "could not preserve existing skill: $skill_target"; }
    printf 'preserved existing skill: %s\n' "$skill_backup"
  fi
  chmod 644 "$skill_temp"
  mv -f "$skill_temp" "$skill_target" || die "could not install skill: $skill_target"
  printf 'registered knapper skill: %s\n' "$skill_target"
}

register_skills() {
  skill_mode="$1"
  registered=0
  case "$skill_mode" in
    none) return 0 ;;
    auto)
      # Only where the host already exists, so installing knapper does not
      # create an agent directory on a machine that has never run one.
      if [ -n "${CODEX_HOME:-}" ] || [ -d "$HOME/.codex" ] || command -v codex >/dev/null 2>&1; then
        install_skill "${CODEX_HOME:-$HOME/.codex}/skills/knapper/SKILL.md"
        registered=1
      fi
      if [ -d "$HOME/.claude" ] || command -v claude >/dev/null 2>&1; then
        install_skill "$HOME/.claude/skills/knapper/SKILL.md"
        registered=1
      fi
      ;;
    codex)
      install_skill "${CODEX_HOME:-$HOME/.codex}/skills/knapper/SKILL.md"
      registered=1
      ;;
    claude)
      install_skill "$HOME/.claude/skills/knapper/SKILL.md"
      registered=1
      ;;
    both)
      install_skill "${CODEX_HOME:-$HOME/.codex}/skills/knapper/SKILL.md"
      install_skill "$HOME/.claude/skills/knapper/SKILL.md"
      registered=1
      ;;
    *) die "invalid skill mode '$skill_mode'; expected auto, none, codex, claude, or both" ;;
  esac
  if [ "$registered" -eq 0 ]; then
    printf 'knapper skill registration skipped; use --skill codex, --skill claude, or --skill both\n'
  fi
}

main() {
  requested_version="latest"
  bin_dir="${KNAPPER_BIN_DIR:-$HOME/.local/bin}"
  skill_mode="auto"
  register_skills_from=""

  while [ "$#" -gt 0 ]; do
    case "$1" in
      --version)
        [ "$#" -ge 2 ] || die "--version requires a value"
        requested_version="$2"
        shift 2
        ;;
      --bin-dir)
        [ "$#" -ge 2 ] || die "--bin-dir requires a value"
        bin_dir="$2"
        shift 2
        ;;
      --skill)
        [ "$#" -ge 2 ] || die "--skill requires a value"
        skill_mode="$2"
        shift 2
        ;;
      --no-skill)
        skill_mode="none"
        shift
        ;;
      --register-skills-from)
        [ "$#" -ge 2 ] || die "--register-skills-from requires a value"
        register_skills_from="$2"
        shift 2
        ;;
      -h|--help)
        usage
        return 0
        ;;
      *) die "unknown option: $1" ;;
    esac
  done

  if [ -n "$register_skills_from" ]; then
    [ -x "$register_skills_from" ] || die "not an executable: $register_skills_from"
    installed_path="$register_skills_from"
    register_skills "$skill_mode"
    return 0
  fi

  need_command curl
  need_command tar

  target="$(detect_target "$(uname -s)" "$(uname -m)")" \
    || die "could not map this machine to a published knapper target"

  release_tag="$(resolve_release_tag "$requested_version")"
  archive_name="$(release_asset_name "$release_tag" "$target")"
  release_base="https://github.com/${KNAPPER_GITHUB_REPO}/releases/download/${release_tag}"
  installed_path="$bin_dir/$KNAPPER_BINARY_NAME"

  temporary_directory="$(mktemp -d)" || die "could not create a temporary directory"
  cleanup() {
    rm -rf "${temporary_directory:-}" 2>/dev/null || true
    if [ -n "${install_temp:-}" ]; then
      rm -f "$install_temp" 2>/dev/null || true
    fi
  }
  trap cleanup 0 1 2 15

  archive_path="$temporary_directory/$archive_name"
  sums_path="$temporary_directory/SHA256SUMS"
  printf 'installing knapper %s for %s\n' "$release_tag" "$target"
  download "$release_base/$archive_name" "$archive_path"
  download "$release_base/SHA256SUMS" "$sums_path"
  verify_checksum "$archive_path" "$sums_path"

  extract_directory="$temporary_directory/extract"
  mkdir -p "$extract_directory"
  tar -xzf "$archive_path" -C "$extract_directory" \
    || die "could not extract knapper archive: $archive_name"
  extracted_binary="$extract_directory/$KNAPPER_BINARY_NAME/$KNAPPER_BINARY_NAME"
  [ -f "$extracted_binary" ] || die "knapper archive did not contain an executable"

  mkdir -p "$bin_dir" || die "could not create install directory: $bin_dir"
  install_temp="$(mktemp "$bin_dir/.knapper.XXXXXX")" \
    || die "could not create a temporary binary in $bin_dir"
  cp "$extracted_binary" "$install_temp" || die "could not copy knapper into $bin_dir"
  chmod 755 "$install_temp"
  mv -f "$install_temp" "$installed_path" || die "could not install knapper into $bin_dir"
  install_temp=""
  printf 'installed knapper at %s\n' "$installed_path"

  register_skills "$skill_mode"
  case ":${PATH:-}:" in
    *":$bin_dir:"*) ;;
    *) printf 'add knapper to future shells with: export PATH="%s:\$PATH"\n' "$bin_dir" ;;
  esac
}

if [ "$KNAPPER_INSTALLER_NO_MAIN" -eq 0 ]; then
  main "$@"
fi
