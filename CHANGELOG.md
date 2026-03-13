# Change Log

## Version 1.0.6

- Fixed an issue where dependencies from non-default features were missing from the generated license file.
- Improved performance by memoizing visited package IDs.

## Version 1.0.5

- Added a `--version` option which displays the `dd-rust-license-tool` version.

## Version 1.0.4

- Added a `--manifest-path` option which allows for running from a different
  directory than the top-level sources.
- Updated a dependency which fixed an issue with missing lines in the generated
  license file.

## Version 1.0.3

- Added support for running in a "pure" workspace.

## Version 1.0.2

- Fixed a crash if a dependency has a directory with any of the names in the list
  of license filenames.
