#!/usr/bin/env bash
# Run from any directory. Never formats files or launches the app.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/check.sh [--with-trash-test]

Run formatting, compilation, workspace tests and Clippy.
By default, exclude restore_roundtrip_through_trash: it uses the session Trash.
Use --with-trash-test only in a disposable user/session with an isolated Trash.
GUI tests may skip without a display; follow docs/VERIFICATION.md separately.
EOF
}

if (( $# > 1 )); then
    usage >&2
    exit 2
fi

test_args=(-- --skip restore_roundtrip_through_trash)
case "${1:-}" in
    "") ;;
    --with-trash-test) test_args=() ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd -- "$project_root"

if ! command -v cargo >/dev/null 2>&1; then
    printf 'Error: cargo is required. See docs/DEVELOPMENT.md.\n' >&2
    exit 1
fi

if (( ${#test_args[@]} )); then
    printf 'Excluding session Trash integration test; see --help.\n'
else
    printf 'Including session Trash integration test (disposable session required).\n'
fi

printf '\n[1/4] Formatting\n'
cargo fmt --check
printf '\n[2/4] Compilation\n'
cargo check --workspace --locked
printf '\n[3/4] Tests\n'
cargo test --workspace --locked "${test_args[@]}"
printf '\n[4/4] Clippy\n'
cargo clippy --workspace --all-targets --locked -- -D warnings
printf '\nAutomated checks passed. Manual GUI verification remains separate.\n'
