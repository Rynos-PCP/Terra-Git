//! Persistence of recently opened repositories (JSON in the app config dir).
//!
//! Format: a list of entries `{path, lastOpened, pinned}`. Older installations
//! have a plain array of paths — the reader migrates that losslessly
//! (lastOpened stays empty until the repo is opened again).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use tauri::{AppHandle, Manager};

const MAX_RECENTS: usize = 15;

/// An entry of the recently-opened list (welcome screen + toolbar menu).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentEntry {
    pub path: String,
    /// Unix seconds of the last open; `None` for migrated legacy entries.
    #[serde(default)]
    pub last_opened: Option<i64>,
    /// Pinned repos come first and never fall out of the list (capping).
    #[serde(default)]
    pub pinned: bool,
}

impl RecentEntry {
    fn from_path(path: String) -> Self {
        Self {
            path,
            last_opened: None,
            pinned: false,
        }
    }
}

/// Tolerant read type: the new object format OR the old path string.
#[derive(Deserialize)]
#[serde(untagged)]
enum StoredRecent {
    Entry(RecentEntry),
    Path(String),
}

fn store_file(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("recent_repos.json"))
}

/// Parses the file content; migrates the old string-array format. Pure — for tests.
pub(crate) fn parse_store(s: &str) -> Vec<RecentEntry> {
    // Tolerate a BOM (e.g. produced by editors/PowerShell).
    serde_json::from_str::<Vec<StoredRecent>>(s.trim_start_matches('\u{feff}'))
        .map(|list| {
            list.into_iter()
                .map(|st| match st {
                    StoredRecent::Entry(e) => e,
                    StoredRecent::Path(p) => RecentEntry::from_path(p),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn read(app: &AppHandle) -> Vec<RecentEntry> {
    let Some(file) = store_file(app) else {
        return Vec::new();
    };
    std::fs::read_to_string(file)
        .ok()
        .map(|s| parse_store(&s))
        .unwrap_or_default()
}

fn write(app: &AppHandle, list: &[RecentEntry]) {
    if let Some(file) = store_file(app) {
        if let Ok(json) = serde_json::to_string_pretty(list) {
            // Atomic (temp+rename): a crash while writing must not truncate the
            // recents — the tolerant reader would otherwise read them as empty.
            if let Err(e) = crate::jsonstore::atomic_write(&file, json.as_bytes()) {
                tracing::warn!("could not write recent_repos.json: {e}");
            }
        }
    }
}

/// Normalizes a repo path for comparison and storage. Without this you get
/// duplicates like `C:/x` vs `C:\x` (the dialog returns forward slashes, git2
/// backslashes) — observed in a real, grown recent_repos.json.
pub(crate) fn canonical_key(p: &str) -> String {
    let s = if cfg!(windows) {
        p.replace('/', "\\")
    } else {
        p.to_string()
    };
    if cfg!(windows) {
        s.to_lowercase() // NTFS is case-insensitive
    } else {
        s
    }
}

/// MRU core of [`add`]: removes duplicates (canonically), inserts the entry at
/// the front with a fresh timestamp and caps at `max`. An existing pin survives
/// the re-add; capping only throws out unpinned entries (from the back).
/// Pure — for tests.
pub(crate) fn push_mru(list: &mut Vec<RecentEntry>, stored: String, now: i64, max: usize) {
    let key = canonical_key(&stored);
    let pinned = list
        .iter()
        .find(|e| canonical_key(&e.path) == key)
        .is_some_and(|e| e.pinned);
    list.retain(|e| canonical_key(&e.path) != key);
    list.insert(
        0,
        RecentEntry {
            path: stored,
            last_opened: Some(now),
            pinned,
        },
    );
    while list.len() > max {
        // Remove the last UNPINNED entry — but never the MRU head just inserted
        // (index 0). If nothing removable is left, the list may exceed the
        // maximum by way of exception.
        let Some(idx) = list.iter().rposition(|e| !e.pinned).filter(|&i| i > 0) else {
            break;
        };
        list.remove(idx);
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Puts `repo_path` at the front of the list (deduplicated, max. 15).
pub fn add(app: &AppHandle, repo_path: &str) {
    let stored = if cfg!(windows) {
        repo_path.replace('/', "\\")
    } else {
        repo_path.to_string()
    };
    let mut list = read(app);
    push_mru(&mut list, stored, unix_now(), MAX_RECENTS);
    write(app, &list);
}

/// Checks (canonically) whether `path` is contained in `list`. Pure — for tests.
pub(crate) fn contains_key(list: &[RecentEntry], path: &str) -> bool {
    let key = canonical_key(path);
    list.iter().any(|e| canonical_key(&e.path) == key)
}

/// Checks whether `path` is in the persisted recents list. Security guard: only
/// known repos may be treated destructively (delete_repo).
pub fn is_known(app: &AppHandle, path: &str) -> bool {
    contains_key(&read(app), path)
}

/// Removes `path` (canonically) from the list. Pure — for tests.
pub(crate) fn remove_key(list: &mut Vec<RecentEntry>, path: &str) {
    let key = canonical_key(path);
    list.retain(|e| canonical_key(&e.path) != key);
}

/// Removes `repo_path` from the persisted list.
pub fn remove(app: &AppHandle, repo_path: &str) {
    let mut list = read(app);
    remove_key(&mut list, repo_path);
    write(app, &list);
}

/// Sets the pinned state (canonically); `true` if an entry was hit.
/// Pure — for tests.
pub(crate) fn set_pinned_key(list: &mut [RecentEntry], path: &str, pinned: bool) -> bool {
    let key = canonical_key(path);
    let mut hit = false;
    for e in list.iter_mut().filter(|e| canonical_key(&e.path) == key) {
        e.pinned = pinned;
        hit = true;
    }
    hit
}

/// Pins or unpins `repo_path` (persisted).
pub fn set_pinned(app: &AppHandle, repo_path: &str, pinned: bool) {
    let mut list = read(app);
    if set_pinned_key(&mut list, repo_path, pinned) {
        write(app, &list);
    }
}

/// Returns the list, filtered down to still-existing directories: pinned
/// entries first, MRU order preserved within each group (stable sort).
pub fn list(app: &AppHandle) -> Vec<RecentEntry> {
    let mut list: Vec<RecentEntry> = read(app)
        .into_iter()
        .filter(|e| std::path::Path::new(&e.path).is_dir())
        .collect();
    list.sort_by_key(|e| !e.pinned);
    list
}

#[cfg(test)]
mod tests {
    use super::{
        contains_key, parse_store, push_mru, remove_key, set_pinned_key, RecentEntry, MAX_RECENTS,
    };

    fn entry(path: &str) -> RecentEntry {
        RecentEntry {
            path: path.to_string(),
            last_opened: None,
            pinned: false,
        }
    }

    fn entries(paths: &[&str]) -> Vec<RecentEntry> {
        paths.iter().map(|p| entry(p)).collect()
    }

    fn paths(list: &[RecentEntry]) -> Vec<String> {
        list.iter().map(|e| e.path.clone()).collect()
    }

    #[test]
    fn parse_store_migrates_old_string_array() {
        let migrated = parse_store(r#"["C:\\a", "C:\\b"]"#);
        assert_eq!(paths(&migrated), vec!["C:\\a", "C:\\b"]);
        // Legacy entries have neither a timestamp nor a pin.
        assert!(migrated
            .iter()
            .all(|e| e.last_opened.is_none() && !e.pinned));
    }

    #[test]
    fn parse_store_reads_new_format_with_bom() {
        let s = "\u{feff}[{\"path\":\"C:\\\\a\",\"lastOpened\":123,\"pinned\":true}]";
        let list = parse_store(s);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].last_opened, Some(123));
        assert!(list[0].pinned);
    }

    #[test]
    fn parse_store_broken_json_empty_list() {
        assert!(parse_store("{not json").is_empty());
    }

    #[test]
    fn push_mru_same_path_different_spelling_one_entry() {
        // Table: (existing entry, new entry in different spelling).
        // Windows dedups slash-/case-insensitively; elsewhere only exact hits.
        let cases: &[(&str, &str)] = if cfg!(windows) {
            &[
                ("C:\\repo", "c:/repo"),
                ("C:\\Repo", "C:\\repo"),
                ("C:/x/y", "c:\\X\\Y"),
            ]
        } else {
            &[("/repo", "/repo")]
        };
        for (old, new) in cases {
            let mut list = vec![entry(old)];
            push_mru(&mut list, new.to_string(), 1, MAX_RECENTS);
            assert_eq!(
                paths(&list),
                vec![new.to_string()],
                "({old}, {new}): expected ONE entry (MRU first)"
            );
        }
    }

    #[test]
    fn push_mru_re_add_moves_to_front_and_stamps() {
        // Table: (new entry, expected order afterwards).
        let cases: &[(&str, &[&str])] = &[
            ("C:\\b", &["C:\\b", "C:\\a", "C:\\c"]),
            ("C:\\c", &["C:\\c", "C:\\a", "C:\\b"]),
            ("C:\\a", &["C:\\a", "C:\\b", "C:\\c"]),
        ];
        for (new, expected) in cases {
            let mut list = entries(&["C:\\a", "C:\\b", "C:\\c"]);
            push_mru(&mut list, new.to_string(), 42, MAX_RECENTS);
            assert_eq!(paths(&list), *expected, "re-add of {new}");
            assert_eq!(list[0].last_opened, Some(42), "timestamp on re-add");
        }
    }

    #[test]
    fn push_mru_keeps_pin_on_re_add() {
        let mut list = entries(&["C:\\a", "C:\\b"]);
        list[1].pinned = true;
        push_mru(&mut list, "C:\\b".to_string(), 7, MAX_RECENTS);
        assert_eq!(paths(&list), vec!["C:\\b", "C:\\a"]);
        assert!(list[0].pinned, "the pin must survive a re-add");
    }

    #[test]
    fn push_mru_caps_at_maximum() {
        // Table: (initial size, maximum) — the new entry goes first, the oldest drops out.
        let cases: &[(usize, usize)] = &[(MAX_RECENTS, MAX_RECENTS), (3, 3), (5, 3)];
        for &(start, max) in cases {
            let mut list: Vec<RecentEntry> =
                (0..start).map(|i| entry(&format!("C:\\r{i}"))).collect();
            push_mru(&mut list, "C:\\new".to_string(), 1, max);
            assert_eq!(list.len(), max, "({start}, {max}): cap violated");
            assert_eq!(list[0].path, "C:\\new", "a new entry must come first");
            assert!(
                !contains_key(&list, &format!("C:\\r{}", start - 1)),
                "({start}, {max}): the oldest entry must drop out"
            );
        }
    }

    #[test]
    fn push_mru_capping_spares_pinned() {
        // The oldest entry is pinned: capping must remove the oldest UNPINNED
        // one instead.
        let mut list = entries(&["C:\\a", "C:\\b", "C:\\c"]);
        list[2].pinned = true;
        push_mru(&mut list, "C:\\new".to_string(), 1, 3);
        assert_eq!(paths(&list), vec!["C:\\new", "C:\\a", "C:\\c"]);

        // Pins only: the list may exceed the maximum rather than lose a pin.
        let mut all_pinned = entries(&["C:\\a", "C:\\b"]);
        for e in &mut all_pinned {
            e.pinned = true;
        }
        push_mru(&mut all_pinned, "C:\\new".to_string(), 1, 2);
        assert_eq!(all_pinned.len(), 3, "pinned entries must not drop out");
    }

    #[test]
    fn set_pinned_key_matches_canonically() {
        let mut list = entries(&["C:\\a", "C:\\b"]);
        let member = if cfg!(windows) { "c:/b" } else { "C:\\b" };
        assert!(set_pinned_key(&mut list, member, true));
        assert!(list[1].pinned);
        assert!(!set_pinned_key(&mut list, "C:\\zzz", true), "non-member");
    }

    #[test]
    fn contains_key_finds_member_canonically() {
        let list = entries(&["C:\\a", "C:\\b"]);
        // Windows: slash- and case-insensitive matches still hit.
        let member = if cfg!(windows) { "c:/b" } else { "C:\\b" };
        assert!(contains_key(&list, member));
    }

    #[test]
    fn contains_key_rejects_non_member() {
        let list = entries(&["C:\\a", "C:\\b"]);
        assert!(!contains_key(&list, "C:\\zzz"));
    }

    #[test]
    fn contains_key_empty_list_false() {
        let list: Vec<RecentEntry> = Vec::new();
        assert!(!contains_key(&list, "C:\\a"));
    }

    #[test]
    fn remove_key_removes_canonically() {
        let mut list = entries(&["C:\\a", "C:\\b", "C:\\c"]);
        // The same path in a different spelling. On Windows canonical_key folds
        // case and slashes, so it hits; everywhere else the path is taken
        // literally and must NOT hit.
        remove_key(&mut list, "c:/b");
        if cfg!(windows) {
            assert_eq!(paths(&list), vec!["C:\\a", "C:\\c"]);
        } else {
            assert_eq!(list.len(), 3, "a different spelling must not match here");
        }
        remove_key(&mut list, "C:\\a");
        assert!(!contains_key(&list, "C:\\a"));
    }
}
