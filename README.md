# Datadog Rust License Tool

This project houses a tool for creating the `LICENSE-3rdparty.csv` file from Rust projects. This
file is required by the Datadog standards for releasing open source code.

## Usage

1. Install the tool using `cargo`:

```
# cargo install --path .
  Installing rust-license-tool v0.1.0 (/home/user/rust-license-tool)
    Updating crates.io index
   Compiling rust-license-tool v0.1.0 (/home/user/rust-license-tool)
    Finished release [optimized] target(s) in 2.56s
  Installing /home/user/.cargo/bin/rust-license-tool
   Installed package `rust-license-tool v0.1.0 (/home/user/rust-license-tool)` (executable `rust-license-tool`)
```

2. In your project directory, create the licenses list file with:
   `rust-license-tool write`.

3. In your CI workflow, check that the licenses list file is up to date with:
   `rust-license-tool check`.

TODO

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
