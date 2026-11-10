# Frozen plan captures

These are `plan` outputs captured from the **version-1 binary** (the `sona`
build, before the document moved to version 2 and before the rename to
`splice`). They exist for one reason: the v2 migration's gate was that a v1
project must produce **byte-identical** plans after the change, for the final,
preview and loudnorm profiles.

`v1-project.json` is still a live fixture — a test opens it through `Store::open`
and asserts the upgrade yields one `main` layer with everything else intact.

The three `v1-*.json` plan captures are **historical**: their argv contains the
old `.sona/` cache path and the old binary name, so they no longer match what the
current build emits, and they are not compared by any test. Do not update them;
if you need a new snapshot, capture one and name it after the version it came
from.
