# changelog.d

Changelog entries waiting for the next release. A change adds its entry here as a file of
its own instead of editing `CHANGELOG.md`, so pull requests no longer conflict with each
other over the changelog (#90).

## Adding an entry

Create a Markdown file in the directory for the type of change:

| Directory     | Heading in CHANGELOG.md |
| ------------- | ----------------------- |
| `added/`      | Added                   |
| `changed/`    | Changed                 |
| `deprecated/` | Deprecated              |
| `removed/`    | Removed                 |
| `fixed/`      | Fixed                   |
| `security/`   | Security                |

Name it after the issue or PR plus a few words, e.g. `fixed/91-text-after-inline-block.md`.
Any name works as long as it is unique; entries whose names start with a number are released
in numeric order.

Write the entry as it should read in CHANGELOG.md, without the leading `- `:

```markdown
Text after an `inline-block` box taller than the text no longer overlaps the box (#91).
When a line holding such a box ended by wrapping or at a `<br>`, the next line started one
text line-height lower instead of below the box.
```

It becomes one list item. A file that starts with `- ` is used as written instead, so one
file can hold several items.

From the repository root, `cargo xtask changelog preview` prints the section the entries would
make, and `cargo xtask changelog check` validates them (CI does too).

## Releasing

In the release commit, run

```sh
cargo xtask changelog release 0.6.0
```

It adds a `## 0.6.0 - <today>` section to CHANGELOG.md holding every entry here, plus anything
still written by hand under `## Unreleased`, and deletes the entry files. `--dry-run` prints
the section without changing anything, and `--date YYYY-MM-DD` sets the date.
