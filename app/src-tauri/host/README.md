# oah-host builds

The release build puts `oah-host-x86_64-linux` and `oah-host-aarch64-linux` here (static musl
builds of `crates/oah-host`), and the app bundles them. oah copies the right one to an SSH host
the first time it's used. To build them locally:

    bun scripts/build-host.ts

Without them the app still works with SSH hosts; it starts one `ssh` per program instead.
