set -o errexit -o xtrace
cargo publish --dry-run
# This errors on a few conditions that the above doesn't, including uncommitted files.
cargo package --list > /dev/null
cargo publish
