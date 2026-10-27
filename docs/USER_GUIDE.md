# User guide

## First import

1. **New import → Source.** Pick a card or folder (system folder picker).
   MediaFlow only reads from it. Give it a name, e.g. “A7S III card A”; it is
   used for the `{camera}` and `{source}` folders.
2. **Destinations & profile.** Pick one or more destination folders. Choose a
   profile (see below) and a project name. Keep “Prove duplicates while
   analyzing” on unless the destination is huge and you are in a hurry.
3. **Preview.** Nothing has been written yet. Check:
   - *to copy* and the space needed on each destination;
   - *already there (verified)*: identical content found at the destination,
     possibly under another name — these are skipped;
   - *conflicts*: a different file already uses the planned name;
   - *renamed*: copied as `name (2).ext` so nothing is overwritten;
   - *excluded*: system files such as `.DS_Store`, with the rule that matched;
   - *not importable*: symbolic links, special or unreadable files.
4. **Import.** Progress is computed from bytes actually copied and re-read.
   The time left appears only once a stable speed is measured.

When the result says **“Import complete — every file is verified”**, every
planned file is at every destination with matching SHA-256. Anything else
(“Finished with problems”, “Interrupted”, “Paused”) means *do not format
the card yet*. Use **Retry failed** or **Resume**.

## Pause, unplug, crash

- **Pause** stops after the current 4 MB block; the file in progress is
  discarded and copied again on resume.
- If a drive is unplugged, the import becomes *Interrupted*. Reconnect it and
  press **Resume**.
- If MediaFlow or the computer stops, the import appears under *Needs
  attention* at next launch. Resume re-checks every file already copied.
- **Cancel** keeps everything already verified and never touches the source.

## Profiles

| Profile | Layout | Notes |
|---|---|---|
| YouTube | `{project}/{date}/{category}` | |
| Professional video | `{project}/{date}/{camera}/{category}` | final re-read of every copy; name conflicts are flagged, not renamed |
| Wedding & events | `{project}/{date}/{camera}/{original_dir}` | final re-read |
| Podcast | `{project}/{category}` | WAV filed as voice-over |
| Photography | `{project}/{year}/{date}` | |
| Custom | `{project}/{original_dir}` | mirrors the card |

`{date}` currently comes from the file's modification date (camera metadata
is not read yet). Categories come from the extension, checked against the
file signature; a mismatch is flagged.

## Verification page

Pick a source and one or more backups. *Verify content* hashes everything and
is the only mode that can declare a backup **complete**. Results per file:
present · verified, different content, corrupted (a copy MediaFlow verified
earlier no longer matches), missing, cannot tell, drive not connected.

## Reports

History → an import → **JSON** / **CSV**. Reports include every operation,
digests, exclusions, problems and the limitations of the verification.
Exports never overwrite an existing file.

## Where is my data?

Settings → Privacy shows the database path. Removing access to a folder in
Settings does not touch any file.
