# Datadog License Tool

This project houses a tool for creating the `LICENSE-3rdparty.csv` file from Rust projects. This
file is required by the Datadog standards for releasing open source code.

## Related Projects

There are a couple of other existing projects that come close to providing the data required for the
above file. However, none of them scan the actual license files within the projects, which is
required to produce the "copyright" field in the file, so all of them would require this as a
follow-on step.

### `cargo-license`

This tool produces most of the required data, and can be run through `jq` to produce a CSV file from
that data. However, it does not report the repository from which the crate came from, so we would
need to parse the `cargo metadata` output anyways.
