# Security policy

## Reporting a vulnerability

Please report it privately, not in a public issue: open a
[security advisory](https://github.com/neuro-kitchen/neuro-convert/security/advisories/new) on this
repository. Include what is affected (a reader, the NWB writer, `verify`, the command line, the
app), the version, and how to reproduce it.

You will get an answer within a week. Fixes are released as a new version and described in the
advisory and the affected crates' `CHANGELOG.md`.

## Supported versions

neuro-convert is in early development (0.x): only the latest release receives fixes.

## Scope

Of particular interest: the readers in `crates/readers/` and the metadata file parser, which read
untrusted recordings and YAML (malformed headers, sizes or offsets that lead to out-of-bounds
reads, huge allocations or hangs on memory-mapped files); the HDF5 and Zarr writers; and anything
that could write recording or subject data somewhere the user did not ask for. If a problem could
expose human-subject data, say so clearly in the report so it can be prioritized.
