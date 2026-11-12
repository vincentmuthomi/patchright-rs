# Vendored Playwright protocol spec

The wire contract this crate implements, taken verbatim from
`packages/protocol/spec/` in the Playwright repository at the tag of the
Playwright release the pinned driver is built on. `DRIVER_VERSION` records
which one.

The driver is `patchright-core`, which numbers its own patch releases and
publishes no spec of its own, so `crates/patchright/build.rs` pins two
versions: `PLAYWRIGHT_VERSION` (the `patchright-core` release) and
`PLAYWRIGHT_UPSTREAM_VERSION` (the Playwright release it is built on, and
the tag this spec comes from). Patchright's own additions to the protocol
(`isolatedContext` on the evaluate commands, `patchrightInitScript` on
`Route.continue`, `focusControl` in the context options) are therefore not
in these files.

It is vendored rather than fetched on demand so that **a driver bump shows
its wire-contract delta as a diff in the bump commit**. That matters
because the driver's validator copies the parameters it knows and drops
the rest: a renamed parameter is not rejected, it is ignored. Playwright
1.63 renamed every `tracingStart` capture parameter, and traces recorded
with the old spellings came out with no DOM to replay and no timeline,
with no error anywhere and every existing test still passing.

## Refreshing it

`cargo xtask update-driver` bumps the driver to the latest stable
`patchright-core` and refreshes the spec with it:

```bash
cargo xtask update-driver          # pin, npm integrity and spec together
git diff protocol-spec             # this diff is the review
```

Read the diff for removed or renamed parameters on commands this crate
sends, then for added ones worth surfacing.

`cargo xtask sync-protocol-spec` refetches the spec alone, at the release
`build.rs` already pins.

`cargo xtask sync-protocol-spec --check` runs offline. It fails when the
vendored copy is not the pinned driver's, and `MANIFEST` lets it also catch
a file that was edited, lost, or added by hand, so "vendored verbatim" is
checked rather than assumed. Nothing runs it automatically in this
repository yet (there is no CI or pre-commit configuration), so run it
before committing a change here. The sync fetches every file before writing
any, so a failed refresh leaves the previous copy intact instead of a
half-written tree the stamp still vouches for.

The `.gitattributes` at the repository root stops Git from converting these
files' line endings on checkout, which would change their digests.

Nothing here is compiled into the library or published; only the crate's
unit tests read these files.
