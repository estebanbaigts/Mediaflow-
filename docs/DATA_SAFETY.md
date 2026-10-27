# Data-loss risks and how MediaFlow handles them

Rule zero: **MediaFlow never deletes, moves or overwrites a user file.** The
only files it ever removes are its own temporary files named `.mfpart-…`,
and only when the journal (or the deterministic name of the file being
re-copied) identifies them as its own.

| Risk | What could go wrong | Handling | Residual risk |
|---|---|---|---|
| Overwrite of an existing file | Same name at destination | Never written over: identical content (SHA-256) → skipped; different → `name (2).ext` or flagged (profile) | none known |
| Race with another program creating the target | Check-then-write window | Kernel no-replace rename; fallback exclusive placeholder | placeholder fallback is two steps (never replaces a foreign file) |
| Partial copy declared complete | Crash, unplug, full disk mid-write | Data written to a temp name; final name only after fsync + re-read + digest match; journal state `done` only after that | none known |
| Silent corruption in transfer | Bad cable, flaky reader, failing flash | Hash while reading source; re-read copy; compare | re-read may hit OS cache (see below) |
| Corruption after import (bit rot) | Drive degrades later | Completeness check (Verification page / `mediaflow verify`) re-hashes and reports `corrupted` | detection only; MediaFlow does not repair |
| Source modified during import | Camera still writing, user editing | Stamp before, after; mismatch → file fails, nothing finalized | a modification that preserves size+mtime+inode during the read window would go unnoticed by the stamp check, but the copy still matches what was read |
| Disconnected drive | Unplugged card/disk | Detected on I/O error by checking the root; session → `interrupted`; resume re-validates disks | — |
| Destination full | Out of space | Pre-check at plan and before each file; ENOSPC fails only that file, temp removed | — |
| Power loss | Lost journal tail | SQLite WAL is always consistent; state changes are checkpointed; resume re-validates every non-final op and re-hashes anything found at a target | drive firmware that ignores flush requests |
| Crash between rename and journal | Final file exists, journal says `verifying` | Resume finds target, hashes it against the source, accepts if identical (no duplicate created) | — |
| Abandoned temp files | Crash during copy | Removed on resume (journaled path) or when the same file is re-copied (deterministic name) | temp files of a session that is never resumed or cancelled remain until then |
| Formatting the card too early | User trusts a partial import | Only `completed` is presented as success; `completed_with_errors` says “do not format the card yet”; report `success` field | user ignores warnings |
| Exclusions hiding files | Profile excludes something important | Exclusions are listed in the plan, the report and the CSV, with the rule and reason | user does not read them |
| Duplicate "cleanup" | Deleting the wrong copy | No deletion feature exists | — |
| Case/Unicode-insensitive file systems | `Clip.MOV` vs `clip.mov`, NFC vs NFD | Collision keys fold case and accents; no-replace rename is the final guard | — |

## The OS-cache caveat

The post-copy verification reads the copy back through the operating system.
Right after writing, that read may be served from RAM (page cache) rather than
from the medium. MediaFlow therefore proves that the OS returns the right
bytes, which catches transfer and logic errors, but not every hardware
write failure. Mitigations today: the `paranoid` profiles re-read at the end of
the session, and a completeness check after re-mounting the drive reads from
the medium. Planned: cache-bypassing verification reads.
