//! Taken from herdr-sidebar — expansion persistence and Collapse All came with it and are unused
//! here, but keeping those parts makes upstream fixes easy to merge. `rescan` is grove's own.
#![allow(dead_code)]
//! Filesystem tree model: which directories are expanded, and the flat list of
//! visible rows the UI renders. Listings are cached so redraws never touch the
//! disk, and `rescan` drops the ones the disk has moved on from.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// One visible line of the tree, in render order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub expanded: bool,
}

/// A cached directory listing, and the directory's mtime as it stood when the
/// listing was read.
struct Listing {
    /// Stamped BEFORE the read, deliberately: a change that lands mid-read then
    /// leaves the stamp looking older than the directory and is caught on the
    /// next scan, where a stamp taken afterwards would swallow it.
    stamp: Option<SystemTime>,
    entries: Vec<Entry>,
}

pub struct Tree {
    root: PathBuf,
    expanded: BTreeSet<PathBuf>,
    cache: HashMap<PathBuf, Listing>,
    pub show_hidden: bool,
}

impl Tree {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            expanded: BTreeSet::new(),
            cache: HashMap::new(),
            show_hidden: true,
        }
    }

    /// The workspace root directory the tree is rooted at.
    pub fn root_path(&self) -> PathBuf {
        self.root.clone()
    }

    /// Display name for the header: the folder's own name, or the full path for
    /// roots like `C:\` that have no final component.
    pub fn root_name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    /// Drop all cached listings; the next `rows()` re-reads the disk.
    pub fn refresh(&mut self) {
        self.cache.clear();
    }

    /// Drop the listing of every visible directory the disk has moved on from,
    /// so the next `rows()` re-reads just those. Reports whether anything was
    /// dropped, so a caller with nothing to do can do nothing.
    ///
    /// A directory's mtime moves when an entry is added, removed or renamed —
    /// exactly the set of changes the tree can show. A write *inside* a file
    /// leaves it alone, and rightly so: the tree shows names, not contents.
    ///
    /// This is a stat per visible directory, not a filesystem watch. It costs
    /// the same on an NFS mount, in a container with no inotify quota left, and
    /// on the far side of `herdr --remote`, which a watch does not.
    pub fn rescan(&mut self) -> bool {
        let moved: Vec<PathBuf> = self
            .cache
            .iter()
            .filter(|(dir, listing)| dir_stamp(dir) != listing.stamp)
            .map(|(dir, _)| dir.clone())
            .collect();
        for dir in &moved {
            self.cache.remove(dir);
        }
        !moved.is_empty()
    }

    /// How many directories `rescan` has to stat — the visible ones.
    pub fn watched_dirs(&self) -> usize {
        self.cache.len()
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    /// The expanded set, for persisting so a sidebar opened in a new tab
    /// comes up showing what the tree already showed.
    pub fn expanded_paths(&self) -> Vec<PathBuf> {
        self.expanded.iter().cloned().collect()
    }

    /// Restore a persisted expanded set. Paths outside this tree's root are
    /// dropped: one state file is shared by every workspace's sidebars.
    pub fn set_expanded(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        self.expanded = paths
            .into_iter()
            .filter(|p| p.starts_with(&self.root))
            .collect();
        self.cache.clear();
    }

    pub fn expand(&mut self, path: &Path) {
        self.expanded.insert(path.to_path_buf());
    }

    pub fn collapse(&mut self, path: &Path) {
        self.expanded.remove(path);
    }

    /// Collapse every expanded directory (the title bar's Collapse All).
    pub fn collapse_all(&mut self) {
        self.expanded.clear();
    }

    pub fn toggle(&mut self, path: &Path) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_path_buf());
        }
    }

    fn children(&mut self, dir: &Path) -> Vec<Entry> {
        if let Some(cached) = self.cache.get(dir) {
            return cached.entries.clone();
        }
        let stamp = dir_stamp(dir);
        let mut entries: Vec<Entry> = fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| Entry {
                        is_dir: e.file_type().map(|t| t.is_dir()).unwrap_or(false),
                        name: e.file_name().to_string_lossy().into_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        sort_entries(&mut entries);
        self.cache.insert(
            dir.to_path_buf(),
            Listing {
                stamp,
                entries: entries.clone(),
            },
        );
        entries
    }

    /// The visible rows, depth-first through expanded directories.
    pub fn rows(&mut self) -> Vec<Row> {
        let mut out = Vec::new();
        let mut walked = HashSet::new();
        let root = self.root.clone();
        self.walk(&root, 0, &mut out, &mut walked);
        // Forget every directory that is no longer on screen. A folded folder's
        // listing is one nobody will read again, and keeping it would have
        // `rescan` stat directories the tree stopped showing hours ago.
        self.cache.retain(|dir, _| walked.contains(dir));
        out
    }

    fn walk(
        &mut self,
        dir: &Path,
        depth: usize,
        out: &mut Vec<Row>,
        walked: &mut HashSet<PathBuf>,
    ) {
        walked.insert(dir.to_path_buf());
        let show_hidden = self.show_hidden;
        for entry in self.children(dir) {
            if !visible(&entry.name, show_hidden) {
                continue;
            }
            let path = dir.join(&entry.name);
            let expanded = entry.is_dir && self.is_expanded(&path);
            out.push(Row {
                name: entry.name,
                is_dir: entry.is_dir,
                depth,
                expanded,
                path: path.clone(),
            });
            if expanded {
                self.walk(&path, depth + 1, out, walked);
            }
        }
    }
}

/// A directory's mtime, or `None` when it cannot be read — which is itself a
/// stable answer, so a directory that has been deleted stops looking changed
/// once its parent has dropped it.
fn dir_stamp(dir: &Path) -> Option<SystemTime> {
    fs::metadata(dir).ok()?.modified().ok()
}

/// VS Code Explorer order: directories first, then files, each case-insensitive.
pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// `.git` is always hidden; other dotfiles only when `show_hidden` is off.
fn visible(name: &str, show_hidden: bool) -> bool {
    name != ".git" && (show_hidden || !name.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("aa-filetree-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn mkdir(&self, rel: &str) {
            fs::create_dir_all(self.0.join(rel)).unwrap();
        }
        fn touch(&self, rel: &str) {
            fs::write(self.0.join(rel), b"").unwrap();
        }
        fn rm(&self, rel: &str) {
            let path = self.0.join(rel);
            if path.is_dir() {
                fs::remove_dir_all(path).unwrap();
            } else {
                fs::remove_file(path).unwrap();
            }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Rescan until the directory mtime has caught up with the change we just
    /// made. Filesystems with sub-second timestamps answer on the first call;
    /// a coarse one costs a tick here instead of a flake in CI.
    fn settled_rescan(tree: &mut Tree) -> bool {
        for _ in 0..40 {
            if tree.rescan() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    }

    fn names(rows: &[Row]) -> Vec<(String, usize)> {
        rows.iter().map(|r| (r.name.clone(), r.depth)).collect()
    }

    #[test]
    fn restored_expansion_ignores_other_workspaces() {
        let tmp = TempDir::new("restore");
        tmp.mkdir("src");
        let mut tree = Tree::new(tmp.0.clone());
        assert!(tree.rows().iter().all(|r| !r.expanded));

        // One state file serves every workspace's sidebars, so a foreign
        // root must be dropped instead of resurrecting as a phantom row.
        tree.set_expanded(vec![
            tmp.0.join("src"),
            PathBuf::from("/somewhere/else/src"),
        ]);
        assert_eq!(tree.expanded_paths(), vec![tmp.0.join("src")]);
        assert!(tree.rows().iter().any(|r| r.name == "src" && r.expanded));
    }

    #[test]
    fn dirs_first_case_insensitive_and_git_hidden() {
        let tmp = TempDir::new("order");
        tmp.mkdir("b_dir");
        tmp.mkdir("A_dir");
        tmp.mkdir(".git");
        tmp.touch("Zebra.txt");
        tmp.touch("apple.rs");
        let mut tree = Tree::new(tmp.0.clone());
        assert_eq!(
            names(&tree.rows()),
            vec![
                ("A_dir".into(), 0),
                ("b_dir".into(), 0),
                ("apple.rs".into(), 0),
                ("Zebra.txt".into(), 0),
            ]
        );
    }

    #[test]
    fn expand_and_collapse_nest_children() {
        let tmp = TempDir::new("expand");
        tmp.mkdir("src");
        tmp.touch("src/main.rs");
        tmp.touch("Cargo.toml");
        let mut tree = Tree::new(tmp.0.clone());
        tree.toggle(&tmp.0.join("src"));
        assert_eq!(
            names(&tree.rows()),
            vec![
                ("src".into(), 0),
                ("main.rs".into(), 1),
                ("Cargo.toml".into(), 0),
            ]
        );
        assert!(tree.rows()[0].expanded);
        tree.toggle(&tmp.0.join("src"));
        assert_eq!(
            names(&tree.rows()),
            vec![("src".into(), 0), ("Cargo.toml".into(), 0)]
        );
    }

    #[test]
    fn collapse_all_closes_every_expanded_dir() {
        let tmp = TempDir::new("collapseall");
        tmp.mkdir("a/inner");
        tmp.mkdir("b");
        let mut tree = Tree::new(tmp.0.clone());
        tree.expand(&tmp.0.join("a"));
        tree.expand(&tmp.0.join("a/inner"));
        tree.expand(&tmp.0.join("b"));
        assert!(tree.rows().iter().any(|r| r.expanded));
        tree.collapse_all();
        assert!(tree.rows().iter().all(|r| !r.expanded));
        assert_eq!(tree.rows().len(), 2, "only the top level remains");
    }

    #[test]
    fn hidden_toggle_filters_dotfiles() {
        let tmp = TempDir::new("hidden");
        tmp.touch(".env");
        tmp.touch("visible.txt");
        let mut tree = Tree::new(tmp.0.clone());
        assert_eq!(tree.rows().len(), 2);
        tree.show_hidden = false;
        assert_eq!(names(&tree.rows()), vec![("visible.txt".into(), 0)]);
    }

    #[test]
    fn refresh_picks_up_new_files() {
        let tmp = TempDir::new("refresh");
        tmp.touch("one.txt");
        let mut tree = Tree::new(tmp.0.clone());
        assert_eq!(tree.rows().len(), 1);
        tmp.touch("two.txt");
        assert_eq!(tree.rows().len(), 1, "cached listing must not re-read disk");
        tree.refresh();
        assert_eq!(tree.rows().len(), 2);
    }

    #[test]
    fn unreadable_or_missing_dir_is_empty() {
        let mut tree = Tree::new(std::env::temp_dir().join("aa-filetree-does-not-exist"));
        assert!(tree.rows().is_empty());
    }

    #[test]
    fn rescan_picks_up_a_file_written_behind_the_tree() {
        let tmp = TempDir::new("rescan-new");
        tmp.touch("one.txt");
        let mut tree = Tree::new(tmp.0.clone());
        assert_eq!(tree.rows().len(), 1);

        // The whole point: nobody pressed a key, an agent just wrote a file.
        tmp.touch("two.txt");
        assert!(settled_rescan(&mut tree));
        assert_eq!(
            names(&tree.rows()),
            vec![("one.txt".into(), 0), ("two.txt".into(), 0)]
        );
    }

    #[test]
    fn rescan_reaches_inside_expanded_directories() {
        let tmp = TempDir::new("rescan-deep");
        tmp.mkdir("src");
        tmp.touch("src/main.rs");
        let mut tree = Tree::new(tmp.0.clone());
        tree.expand(&tmp.0.join("src"));
        assert_eq!(tree.rows().len(), 2);

        tmp.touch("src/lib.rs");
        assert!(settled_rescan(&mut tree));
        assert!(
            tree.rows()
                .iter()
                .any(|r| r.name == "lib.rs" && r.depth == 1)
        );
    }

    #[test]
    fn a_still_tree_reports_no_change_and_stays_cached() {
        let tmp = TempDir::new("rescan-still");
        tmp.mkdir("src");
        tmp.touch("src/main.rs");
        let mut tree = Tree::new(tmp.0.clone());
        tree.expand(&tmp.0.join("src"));
        let before = tree.rows();
        assert!(!tree.rescan(), "nothing moved, so nothing to re-read");
        assert_eq!(tree.rows(), before);
    }

    #[test]
    fn writing_into_a_file_leaves_the_tree_alone() {
        let tmp = TempDir::new("rescan-write");
        tmp.touch("one.txt");
        let mut tree = Tree::new(tmp.0.clone());
        tree.rows();

        // The tree shows names. A file growing is the preview's business, and
        // rebuilding the rows for it would be churn nobody can see.
        fs::write(tmp.0.join("one.txt"), b"now with contents").unwrap();
        assert!(!tree.rescan());
    }

    #[test]
    fn a_deleted_entry_leaves_the_tree() {
        let tmp = TempDir::new("rescan-gone");
        tmp.mkdir("doomed");
        tmp.touch("doomed/child.txt");
        tmp.touch("kept.txt");
        let mut tree = Tree::new(tmp.0.clone());
        tree.expand(&tmp.0.join("doomed"));
        assert_eq!(tree.rows().len(), 3);

        tmp.rm("doomed");
        assert!(settled_rescan(&mut tree));
        assert_eq!(names(&tree.rows()), vec![("kept.txt".into(), 0)]);
        // And the vanished directory stops costing a stat every tick.
        assert_eq!(tree.watched_dirs(), 1, "only the root is left to watch");
        assert!(!tree.rescan(), "a deleted directory must not keep flapping");
    }

    #[test]
    fn folding_a_directory_stops_it_being_watched() {
        let tmp = TempDir::new("rescan-fold");
        tmp.mkdir("a/inner");
        tmp.mkdir("b");
        let mut tree = Tree::new(tmp.0.clone());
        tree.expand(&tmp.0.join("a"));
        tree.expand(&tmp.0.join("a/inner"));
        tree.rows();
        assert_eq!(tree.watched_dirs(), 3, "root, a, a/inner");

        tree.collapse(&tmp.0.join("a"));
        tree.rows();
        assert_eq!(tree.watched_dirs(), 1, "only the root is still on screen");
    }

    #[test]
    fn root_name_uses_final_component() {
        let tmp = TempDir::new("rootname");
        let tree = Tree::new(tmp.0.clone());
        assert!(tree.root_name().starts_with("aa-filetree-"));
    }
}
