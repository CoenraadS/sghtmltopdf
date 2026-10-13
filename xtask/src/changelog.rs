//! `cargo xtask changelog`: one file per changelog entry.
//!
//! Every PR used to add its entry under `## Unreleased` in CHANGELOG.md, so as soon as one PR
//! was merged, every other open PR conflicted there (#90). Instead, a PR adds its entry as a
//! file of its own, `changelog.d/<category>/<name>.md`, and the release commit gathers those
//! files into a version section of CHANGELOG.md and deletes them. Two PRs never touch the
//! same file, so they cannot conflict.
//!
//! Entries already written by hand under `## Unreleased` are released together with the
//! files, so PRs opened before the switch need no changes.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Where the entries live, relative to the repository root.
pub const ENTRY_DIR: &str = "changelog.d";

/// Keep a Changelog's types of change, in the order they appear in a release: the
/// directory under `changelog.d/`, and the heading in CHANGELOG.md.
pub const CATEGORIES: [(&str, &str); 6] = [
    ("added", "Added"),
    ("changed", "Changed"),
    ("deprecated", "Deprecated"),
    ("removed", "Removed"),
    ("fixed", "Fixed"),
    ("security", "Security"),
];

/// One file under `changelog.d/<category>/`.
#[derive(Debug)]
pub struct Entry {
    /// The path relative to the repository root, for messages.
    pub name: String,
    pub path: PathBuf,
    /// Index into [`CATEGORIES`].
    pub category: usize,
    /// The contents with line endings normalised and surrounding whitespace trimmed.
    pub text: String,
}

/// `cargo xtask changelog check`.
pub fn check(root: &Path) -> Result<String, String> {
    let entries = read_entries(root).map_err(|problems| problems.join("\n"))?;
    Ok(format!(
        "{ENTRY_DIR}/: {} entries, all valid",
        entries.len()
    ))
}

/// `cargo xtask changelog preview`.
pub fn preview(root: &Path) -> Result<String, String> {
    let entries = read_entries(root).map_err(|problems| problems.join("\n"))?;
    let changelog = read_changelog(root)?.replace("\r\n", "\n");
    let unreleased = split(&changelog).unreleased;
    Ok(section("## Unreleased", unreleased, &entries)
        .unwrap_or_else(|| "Nothing is waiting to be released.".to_string()))
}

/// `cargo xtask changelog release`. Returns the new section.
pub fn release(root: &Path, version: &str, date: &str, dry_run: bool) -> Result<String, String> {
    let version = version.strip_prefix('v').unwrap_or(version);
    let valid_version = version.starts_with(|c: char| c.is_ascii_digit())
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    if !valid_version {
        return Err(format!(
            "{version:?} does not look like a version (e.g. 0.6.0)"
        ));
    }
    let valid_date = date.len() == 10
        && date.char_indices().all(|(i, c)| match i {
            4 | 7 => c == '-',
            _ => c.is_ascii_digit(),
        });
    if !valid_date {
        return Err(format!("{date:?} is not a YYYY-MM-DD date"));
    }

    let entries = read_entries(root).map_err(|problems| problems.join("\n"))?;
    let original = read_changelog(root)?;
    let changelog = original.replace("\r\n", "\n");
    if released_versions(&changelog).any(|released| released == version) {
        return Err(format!("CHANGELOG.md already has a {version} section"));
    }
    let heading = format!("## {version} - {date}");
    let Some((updated, section)) = release_text(&changelog, &heading, &entries) else {
        return Err(format!(
            "nothing to release: {ENTRY_DIR}/ has no entries and CHANGELOG.md has no \
             `## Unreleased` section"
        ));
    };
    if dry_run {
        return Ok(section);
    }

    // Keep the line endings the file was checked out with.
    let updated = if original.contains("\r\n") {
        updated.replace('\n', "\r\n")
    } else {
        updated
    };
    fs::write(root.join("CHANGELOG.md"), updated)
        .map_err(|e| format!("could not write CHANGELOG.md: {e}"))?;
    for entry in &entries {
        fs::remove_file(&entry.path)
            .map_err(|e| format!("could not delete {}: {e}", entry.name))?;
    }
    // Only an emptied directory goes; one still holding something (a `.gitkeep`) stays.
    for (dir, _) in CATEGORIES {
        let _ = fs::remove_dir(root.join(ENTRY_DIR).join(dir));
    }
    Ok(format!(
        "{section}\nAdded the {version} section to CHANGELOG.md and deleted {} entries from \
         {ENTRY_DIR}/.",
        entries.len()
    ))
}

fn read_changelog(root: &Path) -> Result<String, String> {
    fs::read_to_string(root.join("CHANGELOG.md"))
        .map_err(|e| format!("could not read CHANGELOG.md: {e}"))
}

/// Reads every entry under `changelog.d/`, or reports everything wrong with it at once.
/// Entries come out by category, then numerically by a leading number in the file name
/// (the issue or PR), then by name.
pub fn read_entries(root: &Path) -> Result<Vec<Entry>, Vec<String>> {
    let dir = root.join(ENTRY_DIR);
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    let expected = CATEGORIES.map(|(dir, _)| dir).join(", ");

    let top = match list_dir(&dir) {
        Ok(top) => top,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(entries),
        Err(e) => return Err(vec![format!("could not read {ENTRY_DIR}/: {e}")]),
    };
    for (name, path, is_dir) in top {
        if name.starts_with('.') || (name == "README.md" && !is_dir) {
            continue;
        }
        if !is_dir {
            problems.push(format!(
                "{ENTRY_DIR}/{name}: put it in the directory for its type of change, \
                 e.g. {ENTRY_DIR}/fixed/{name}"
            ));
            continue;
        }
        let Some(category) = CATEGORIES.iter().position(|(dir, _)| *dir == name) else {
            problems.push(format!(
                "{ENTRY_DIR}/{name}/: not a type of change (expected one of: {expected})"
            ));
            continue;
        };
        let files = match list_dir(&path) {
            Ok(files) => files,
            Err(e) => {
                problems.push(format!("could not read {ENTRY_DIR}/{name}/: {e}"));
                continue;
            }
        };
        for (file, path, is_dir) in files {
            let entry_name = format!("{ENTRY_DIR}/{name}/{file}");
            if file.starts_with('.') {
                continue;
            }
            if is_dir || !file.ends_with(".md") {
                problems.push(format!("{entry_name}: entries are .md files"));
                continue;
            }
            match fs::read_to_string(&path) {
                Ok(text) => {
                    let text = text.replace("\r\n", "\n").trim().to_string();
                    if text.is_empty() {
                        problems.push(format!("{entry_name}: empty"));
                    } else {
                        entries.push(Entry {
                            name: entry_name,
                            path,
                            category,
                            text,
                        });
                    }
                }
                Err(e) => problems.push(format!("{entry_name}: {e}")),
            }
        }
    }

    if !problems.is_empty() {
        return Err(problems);
    }
    entries.sort_by(|a, b| (a.category, sort_key(&a.name)).cmp(&(b.category, sort_key(&b.name))));
    Ok(entries)
}

/// The directory's children as `(name, path, is_dir)`, sorted by name.
fn list_dir(dir: &Path) -> io::Result<Vec<(String, PathBuf, bool)>> {
    let mut children = Vec::new();
    for child in fs::read_dir(dir)? {
        let child = child?;
        let name = child.file_name().to_string_lossy().into_owned();
        children.push((name, child.path(), child.file_type()?.is_dir()));
    }
    children.sort();
    Ok(children)
}

/// `changelog.d/fixed/91-foo.md` sorts as 91, before `100-bar.md`. A name without a number
/// sorts after the numbered ones.
fn sort_key(name: &str) -> (u64, &str) {
    let file = name.rsplit('/').next().unwrap_or(name);
    let digits = file.len() - file.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    (file[..digits].parse().unwrap_or(u64::MAX), file)
}

/// CHANGELOG.md cut around the section a release replaces.
#[derive(Debug, PartialEq)]
struct Split<'a> {
    /// Everything before `## Unreleased`, or before the latest release if there is no
    /// Unreleased section.
    head: &'a str,
    /// The body of `## Unreleased` without the heading; empty if there is none.
    unreleased: &'a str,
    /// The latest release onwards.
    tail: &'a str,
}

fn split(changelog: &str) -> Split<'_> {
    // (start of the heading, start of the body)
    let mut unreleased: Option<(usize, usize)> = None;
    let mut offset = 0;
    for line in changelog.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        if !line.starts_with("## ") {
            continue;
        }
        match unreleased {
            Some((heading, body)) => {
                return Split {
                    head: &changelog[..heading],
                    unreleased: &changelog[body..start],
                    tail: &changelog[start..],
                }
            }
            None if is_unreleased_heading(line) => unreleased = Some((start, offset)),
            None => {
                return Split {
                    head: &changelog[..start],
                    unreleased: "",
                    tail: &changelog[start..],
                }
            }
        }
    }
    match unreleased {
        Some((heading, body)) => Split {
            head: &changelog[..heading],
            unreleased: &changelog[body..],
            tail: "",
        },
        None => Split {
            head: changelog,
            unreleased: "",
            tail: "",
        },
    }
}

/// `## Unreleased`, or Keep a Changelog's `## [Unreleased]`.
fn is_unreleased_heading(line: &str) -> bool {
    let title = line["## ".len()..].trim();
    let title = title
        .strip_prefix('[')
        .and_then(|title| title.strip_suffix(']'))
        .unwrap_or(title);
    title.eq_ignore_ascii_case("unreleased")
}

/// The versions CHANGELOG.md already has a section for (`## 0.5.1 - 2026-09-23` → `0.5.1`).
fn released_versions(changelog: &str) -> impl Iterator<Item = &str> {
    changelog.lines().filter_map(|line| {
        let title = line.strip_prefix("## ")?.trim_start_matches('[');
        title.split([' ', ']']).next()
    })
}

/// CHANGELOG.md with the Unreleased section (if any) and the entries released under
/// `heading`, and that section alone. `None` when there is nothing to release.
fn release_text(changelog: &str, heading: &str, entries: &[Entry]) -> Option<(String, String)> {
    let split = split(changelog);
    let section = section(heading, split.unreleased, entries)?;
    let head = split.head.trim_end();
    let mut updated = String::new();
    if !head.is_empty() {
        updated.push_str(head);
        updated.push_str("\n\n");
    }
    updated.push_str(&section);
    if !split.tail.is_empty() {
        updated.push('\n');
        updated.push_str(split.tail);
    }
    Some((updated, section))
}

/// The section `heading` that releases the hand-written Unreleased entries followed by the
/// files, one `###` group per type of change. `None` when both are empty.
fn section(heading: &str, unreleased: &str, entries: &[Entry]) -> Option<String> {
    let mut groups = groups(unreleased);
    let preamble = match groups.first() {
        Some((name, _)) if name.is_empty() => Some(groups.remove(0).1),
        _ => None,
    };

    let mut bodies: Vec<(String, Vec<String>)> = Vec::new();
    for (category, (_, title)) in CATEGORIES.iter().enumerate() {
        let mut items = Vec::new();
        if let Some(i) = groups
            .iter()
            .position(|(name, _)| name.eq_ignore_ascii_case(title))
        {
            items.push(groups.remove(i).1);
        }
        items.extend(
            entries
                .iter()
                .filter(|entry| entry.category == category)
                .map(|entry| list_item(&entry.text)),
        );
        if !items.is_empty() {
            bodies.push((title.to_string(), items));
        }
    }
    // A hand-written group that is not one of Keep a Changelog's types stays, after them.
    bodies.extend(groups.into_iter().map(|(name, text)| (name, vec![text])));

    if preamble.is_none() && bodies.is_empty() {
        return None;
    }
    let mut section = format!("{heading}\n\n");
    if let Some(preamble) = preamble {
        section.push_str(&preamble);
        section.push_str("\n\n");
    }
    for (name, items) in bodies {
        section.push_str(&format!("### {name}\n\n{}\n\n", join_items(&items)));
    }
    section.truncate(section.trim_end().len());
    section.push('\n');
    Some(section)
}

/// A hand-written section's `### <type>` groups, in order and with empty ones dropped. Text
/// before the first heading comes first, under an empty name.
fn groups(body: &str) -> Vec<(String, String)> {
    let mut groups = vec![(String::new(), String::new())];
    for line in body.lines() {
        if let Some(name) = line.strip_prefix("### ") {
            groups.push((name.trim().to_string(), String::new()));
        } else {
            let text = &mut groups.last_mut().expect("starts non-empty").1;
            text.push_str(line);
            text.push('\n');
        }
    }
    groups
        .into_iter()
        .map(|(name, text)| (name, text.trim().to_string()))
        .filter(|(_, text)| !text.is_empty())
        .collect()
}

/// An entry as a list item: the bullet on its first line and the rest indented under it. A
/// file that already starts with `- ` is used as written, so one file can hold several items.
fn list_item(text: &str) -> String {
    if text.starts_with("- ") {
        return text.to_string();
    }
    let mut item = String::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 {
            item.push_str("- ");
        } else {
            item.push('\n');
            if !line.is_empty() {
                item.push_str("  ");
            }
        }
        item.push_str(line);
    }
    item
}

/// One item per line, as CHANGELOG.md has them, except that an item of several paragraphs
/// gets a blank line on each side so the next item does not read as one of its paragraphs.
fn join_items(items: &[String]) -> String {
    let mut joined = String::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            let loose = item.contains("\n\n") || items[i - 1].contains("\n\n");
            joined.push_str(if loose { "\n\n" } else { "\n" });
        }
        joined.push_str(item);
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str =
        "# CHANGELOG\n\nAll notable changes to this project will be documented in this file.\n\n";
    const RELEASED: &str = "## 0.5.1 - 2026-09-23\n\n### Added\n\n- Something old.\n";

    fn entry(category: &str, name: &str, text: &str) -> Entry {
        Entry {
            name: format!("{ENTRY_DIR}/{category}/{name}"),
            path: PathBuf::new(),
            category: CATEGORIES
                .iter()
                .position(|(dir, _)| *dir == category)
                .unwrap(),
            text: text.to_string(),
        }
    }

    /// A scratch repository root that is removed again when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("xtask-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn an_entry_becomes_one_list_item() {
        assert_eq!(list_item("One line."), "- One line.");
        assert_eq!(
            list_item("First line\ncontinued.\n\nSecond paragraph."),
            "- First line\n  continued.\n\n  Second paragraph."
        );
        assert_eq!(
            list_item("- Already\n  a list.\n- Two."),
            "- Already\n  a list.\n- Two."
        );
    }

    #[test]
    fn items_of_several_paragraphs_are_set_apart() {
        let items = ["- a".to_string(), "- b".to_string()];
        assert_eq!(join_items(&items), "- a\n- b");
        let items = [
            "- a".to_string(),
            "- b\n\n  more".to_string(),
            "- c".to_string(),
        ];
        assert_eq!(join_items(&items), "- a\n\n- b\n\n  more\n\n- c");
    }

    #[test]
    fn entries_go_above_the_latest_release() {
        let changelog = format!("{HEADER}{RELEASED}");
        let entries = [
            entry("added", "1-a.md", "New thing (#1)."),
            entry("fixed", "2-b.md", "Broken thing (#2)."),
        ];
        let (updated, section) =
            release_text(&changelog, "## 0.6.0 - 2026-10-13", &entries).unwrap();
        let expected_section = "## 0.6.0 - 2026-10-13\n\n### Added\n\n- New thing (#1).\n\n### Fixed\n\n- Broken thing (#2).\n";
        assert_eq!(section, expected_section);
        assert_eq!(updated, format!("{HEADER}{expected_section}\n{RELEASED}"));
    }

    #[test]
    fn hand_written_unreleased_entries_are_released_with_the_files() {
        let changelog = format!(
            "{HEADER}## Unreleased\n\n### Fixed\n\n- Written by hand.\n\n### Added\n\n- Also by hand.\n\n### Notes\n\n- Not a Keep a Changelog type.\n\n{RELEASED}"
        );
        let entries = [
            entry("fixed", "3-c.md", "From a file."),
            entry("removed", "4-d.md", "Gone."),
        ];
        let (updated, section) =
            release_text(&changelog, "## 0.6.0 - 2026-10-13", &entries).unwrap();
        let expected_section = "## 0.6.0 - 2026-10-13\n\n\
            ### Added\n\n- Also by hand.\n\n\
            ### Removed\n\n- Gone.\n\n\
            ### Fixed\n\n- Written by hand.\n- From a file.\n\n\
            ### Notes\n\n- Not a Keep a Changelog type.\n";
        assert_eq!(section, expected_section);
        assert_eq!(updated, format!("{HEADER}{expected_section}\n{RELEASED}"));
    }

    #[test]
    fn an_unreleased_section_alone_is_released_as_is() {
        let changelog = format!("{HEADER}## [Unreleased]\n\n### Fixed\n\n- By hand.\n\n{RELEASED}");
        let (updated, _) = release_text(&changelog, "## 0.6.0 - 2026-10-13", &[]).unwrap();
        assert_eq!(
            updated,
            format!("{HEADER}## 0.6.0 - 2026-10-13\n\n### Fixed\n\n- By hand.\n\n{RELEASED}")
        );
    }

    #[test]
    fn nothing_to_release() {
        let changelog = format!("{HEADER}## Unreleased\n\n### Fixed\n\n{RELEASED}");
        assert_eq!(release_text(&changelog, "## 0.6.0", &[]), None);
        assert_eq!(
            release_text(&format!("{HEADER}{RELEASED}"), "## 0.6.0", &[]),
            None
        );
    }

    #[test]
    fn the_first_release_goes_after_the_header() {
        let entries = [entry("added", "a.md", "Everything.")];
        let (updated, _) = release_text(HEADER, "## 0.1.0 - 2026-01-01", &entries).unwrap();
        assert_eq!(
            updated,
            format!("{HEADER}## 0.1.0 - 2026-01-01\n\n### Added\n\n- Everything.\n")
        );
    }

    #[test]
    fn finds_the_released_versions() {
        let changelog = format!("{HEADER}## [0.6.0] - 2026-10-13\n\n{RELEASED}## 0.5.0\n");
        let versions: Vec<_> = released_versions(&changelog).collect();
        assert_eq!(versions, ["0.6.0", "0.5.1", "0.5.0"]);
    }

    #[test]
    fn reads_entries_in_order() {
        let scratch = Scratch::new("order");
        scratch.write("changelog.d/README.md", "Not an entry.");
        scratch.write("changelog.d/.gitkeep", "");
        scratch.write("changelog.d/fixed/100-later.md", "Hundred.");
        scratch.write("changelog.d/fixed/91-earlier.md", "Ninety-one.\r\n");
        scratch.write("changelog.d/fixed/no-number.md", "Unnumbered.");
        scratch.write("changelog.d/added/7-new.md", "Seven.");
        let entries = read_entries(&scratch.0).unwrap();
        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "changelog.d/added/7-new.md",
                "changelog.d/fixed/91-earlier.md",
                "changelog.d/fixed/100-later.md",
                "changelog.d/fixed/no-number.md",
            ]
        );
        assert_eq!(entries[1].text, "Ninety-one.");
    }

    #[test]
    fn reports_every_misplaced_file() {
        let scratch = Scratch::new("problems");
        scratch.write("changelog.d/stray.md", "Where does this go?");
        scratch.write("changelog.d/fix/1-typo.md", "Misspelt directory.");
        scratch.write("changelog.d/fixed/2-empty.md", " \n\n");
        scratch.write("changelog.d/fixed/3-notes.txt", "Not markdown.");
        let problems = read_entries(&scratch.0).unwrap_err();
        assert_eq!(problems.len(), 4, "{problems:#?}");
        assert!(problems[0].starts_with("changelog.d/fix/: not a type of change"));
        assert!(problems[1].starts_with("changelog.d/fixed/2-empty.md: empty"));
        assert!(problems[2].starts_with("changelog.d/fixed/3-notes.txt: entries are .md files"));
        assert!(problems[3].starts_with("changelog.d/stray.md: put it in the directory"));
    }

    #[test]
    fn release_writes_the_section_and_deletes_the_files() {
        let scratch = Scratch::new("release");
        scratch.write(
            "CHANGELOG.md",
            &format!("{HEADER}{RELEASED}").replace('\n', "\r\n"),
        );
        scratch.write("changelog.d/README.md", "Stays.");
        scratch.write("changelog.d/fixed/5-e.md", "Fixed it.");

        let err = release(&scratch.0, "0.5.1", "2026-10-13", false).unwrap_err();
        assert_eq!(err, "CHANGELOG.md already has a 0.5.1 section");
        assert!(release(&scratch.0, "0.6.0", "13 October", false).is_err());

        release(&scratch.0, "0.6.0", "2026-10-13", true).unwrap();
        assert!(
            scratch.0.join("changelog.d/fixed/5-e.md").exists(),
            "--dry-run deletes nothing"
        );

        release(&scratch.0, "v0.6.0", "2026-10-13", false).unwrap();
        let changelog = fs::read_to_string(scratch.0.join("CHANGELOG.md")).unwrap();
        let expected =
            format!("{HEADER}## 0.6.0 - 2026-10-13\n\n### Fixed\n\n- Fixed it.\n\n{RELEASED}");
        assert_eq!(
            changelog,
            expected.replace('\n', "\r\n"),
            "keeps CRLF line endings"
        );
        assert!(!scratch.0.join("changelog.d/fixed").exists());
        assert!(scratch.0.join("changelog.d/README.md").exists());

        let err = release(&scratch.0, "0.6.1", "2026-10-14", false).unwrap_err();
        assert!(err.starts_with("nothing to release"), "{err}");
    }

    /// The entries in this repository, so CI fails on a misplaced one rather than the release.
    #[test]
    fn this_repository_is_releasable() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        if let Err(problems) = read_entries(root) {
            panic!("{}", problems.join("\n"));
        }
        preview(root).unwrap();
    }
}
