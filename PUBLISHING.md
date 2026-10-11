# Publishing VUI

VUI crates share one version and are published in dependency order:

1. `vui-core`
2. `vui-accesskit`, `vui-platform`, `vui-resources`, `vui-session`, `vui-testing`, `vui-text`, `vui-vello`
3. `vui-winit` and `vui-snapshot`
4. `vui-platform-linux`

Before tagging a release:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
VUI_REQUIRE_GPU_SNAPSHOTS=1 cargo test --workspace --all-features
./scripts/package.sh
```

`scripts/package.sh` uses Cargo's workspace packaging mode, which builds a
temporary local registry and verifies every generated archive against packaged
workspace dependencies. During an actual release, publish each tier, wait for
the crates.io index, and run `cargo publish --dry-run` before publishing the
next tier.

API compatibility CI compares pull requests against the `v0.1.0` tag. Move the
baseline tag in the workflow only when starting a deliberately incompatible
release line.
