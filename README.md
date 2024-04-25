# Datadog Rust License Tool

This project houses a tool for creating the `LICENSE-3rdparty.csv` file from Rust projects. This
file is required by the Datadog standards for releasing open source code.

## Usage

1. Install the tool using `cargo`: `cargo install dd-rust-license-tool`

2. In your project directory, create the licenses list file with:
   `dd-rust-license-tool write`.

3. In your CI workflow, check that the licenses list file is up to date with:
   `dd-rust-license-tool check`.

Additionally you can pass the the following arguments to get the licenses for optional dependencies
behind features:

* `--features []`, `-f []`: comma separated list of features.
* `--all-features`: activate all feature dependencies.

## Configuration

The license tool loads a configuration file at startup that may contain overrides or supplementary
data for packages. This can be useful where a crate does not supply either a homepage or repository
URL, or is missing an explicit license. The filename of this configuration file defaults to
`license-tool.toml` but can be overridden with the `--config` command-line option.

Example:

```toml
[overrides]
# These crates do not specify a homepage in their metadata.

"openssl-macros" = { origin = "https://github.com/sfackler/rust-openssl" }
"serde_nanos" = { origin = "https://github.com/caspervonb/serde_nanos" }

# `zerocopy` et al don't specify their licenses in the metadata, but the file contains the 2-clause
# BSD terms. These should use versioned identifiers, as they could change from version to version
# and so need to be reviewed after each version bump.

"zerocopy-0.6.1" = { license = "BSD-2-Clause" }
"zerocopy-derive-0.3.2" = { license = "BSD-2-Clause" }
```

## Related Projects

There are other existing projects that come close to providing the data required for the above
file. However, none of them scan the actual license or source files within the projects, which is
required to produce the "copyright" field in the file, so all of them would require this as a
follow-on step. Most also do not report the repository from which the crate came from, so we would
need to parse the `cargo metadata` output anyways. None have options to output into CSV, and so
additionally require a post-processing step.

### `cargo-about`

Has integrated license validity checking.

### `cargo-bundle-licenses`

Similar kind of tool to this one, with all the limitations above.

### `cargo-deny`

Groups all results on the licenses rather than listing all the licenses per dependency, making it
impossible to generate an accurate CSV listing.

### `cargo-license`

Limitations as above.

### `extrude-licenses`

Is just a wrapper for `cargo-license`, so has all its limitations.
