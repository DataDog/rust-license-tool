if [ $# -ne 1 ]
then
  echo "$0 VERSION"
  exit 1
fi
version=$1
set -o errexit

check_files() {
  if ! grep --quiet --fixed-strings --line-regexp "version = \"$version\"" Cargo.toml
  then
    echo "Make sure the version has been bumped in Cargo.toml"
    exit 1
  fi

  if ! grep --quiet --fixed-strings --line-regexp "## Version $version" CHANGELOG.md
  then
    echo "Make sure the change log has an entry for this version."
    exit 1
  fi
}

check_files

if [ -n "$( git status --porcelain=v1 )" ]
then
  echo "Error: local tree has modified files."
  git status
  exit 1
fi

git fetch origin
git checkout origin/main

# Sanity check to ensure `main` has the same release tags
check_files

cargo publish --dry-run
# This errors on a few conditions that the above doesn't, including uncommitted files.
cargo package --list > /dev/null

echo "Press <Enter> to continue to publish version $version to GitHub and crates.io."
read

git tag v$version
git push origin v$version

echo "Waiting for release workflow to start (timeout: 10m)..."
run_id=""
deadline=$(( $(date +%s) + 600 ))
while [ -z "$run_id" ]; do
  if [ "$(date +%s)" -ge "$deadline" ]; then
    echo "Error: release workflow did not start within 10 minutes."
    exit 1
  fi
  sleep 5
  run_id=$(gh run list --workflow=release.yml --branch="v$version" \
    --json databaseId --jq '.[0].databaseId' 2>/dev/null || true)
done

echo "Release workflow started (run $run_id). Waiting for it to finish..."
gh run watch "$run_id" --exit-status
echo "Release workflow completed successfully."

echo "Publishing to crates.io..."
cargo publish
echo "Done. Version $version published."
