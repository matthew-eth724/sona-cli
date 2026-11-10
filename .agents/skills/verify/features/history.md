# History and undo

## Sub-features

- Every mutating command snapshots `sona.json` into `.sona/history/` before it
  changes anything.
- `history` lists the snapshots newest first, labelled with the command.
- `undo [--steps N]` restores the previous document and drops the snapshot.

## How to get to it (user POV)

```bash
$SONA add clip-a.mp4
$SONA text "oops" --from 0 --to 2
$SONA history | jq '.undoable, [.entries[].command]'
$SONA undo
$SONA list | jq -r '.clips | length'
```

## Driving it with bash + JSON

```bash
before=$($SONA list | jq -r '.duration')
$SONA trim clip_1 --to 2s > /dev/null
$SONA undo | jq '.restored[0].command'      # "trim"
after=$($SONA list | jq -r '.duration')
[ "$before" = "$after" ] && echo "restored exactly"
```

Recovery check, the reason this exists:

```bash
echo '{ broken' > sona.json
$SONA undo                                   # does not parse the live file
$SONA list | jq -r '.clips | length'
```

## Gotchas

- `undo` does not parse `sona.json`, so it works on a corrupt document — that is
  the case it is built for. Do not "fix" it by opening the store first.
- Snapshots that would restore an identical document are skipped and consumed,
  so a command that failed after taking one does not cost an undo. A `history`
  entry therefore may not correspond to a real change.
- The stack is capped at 50 entries; older ones are pruned. Long sessions lose
  the far past first.
- `cut --dry-run` takes no snapshot (it changes nothing); a real `cut` does.
- `.sona/` is normally disposable cache. Deleting it mid-session throws the undo
  stack away, which is the one thing you cannot re-derive.
