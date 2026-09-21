use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_DEPTH: usize = 32;
const MAX_ENTRIES: usize = 20_000;

#[derive(Clone, Debug)]
pub struct WorkspaceEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub children: Vec<WorkspaceEntry>,
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub root: PathBuf,
    pub entries: Vec<WorkspaceEntry>,
    pub truncated: bool,
    pub skipped_directories: usize,
}

impl Workspace {
    pub fn open(root: PathBuf) -> Result<Self, String> {
        if !root.is_dir() {
            return Err(format!("工作区不是目录：{}", root.display()));
        }
        let mut count = 0;
        let mut truncated = false;
        let mut skipped_directories = 0;
        let entries = scan_directory(
            &root,
            0,
            &mut count,
            &mut truncated,
            &mut skipped_directories,
        )?;
        Ok(Self {
            root,
            entries,
            truncated,
            skipped_directories,
        })
    }

    pub fn refresh(&mut self) -> Result<(), String> {
        let refreshed = Self::open(self.root.clone())?;
        *self = refreshed;
        Ok(())
    }
}

fn scan_directory(
    directory: &Path,
    depth: usize,
    count: &mut usize,
    truncated: &mut bool,
    skipped_directories: &mut usize,
) -> Result<Vec<WorkspaceEntry>, String> {
    if depth >= MAX_DEPTH || *count >= MAX_ENTRIES {
        *truncated = true;
        return Ok(Vec::new());
    }

    let read_dir = fs::read_dir(directory)
        .map_err(|error| format!("无法读取目录 {}：{error}", directory.display()))?;
    let mut entries = Vec::new();
    for result in read_dir {
        if *count >= MAX_ENTRIES {
            *truncated = true;
            break;
        }
        // Bound filesystem work, including entries that will not be shown.
        // Charge directories before recursion so ancestors cannot exceed it.
        *count += 1;
        let Ok(entry) = result else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if should_ignore(&name) {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }

        if metadata.is_dir() {
            let children =
                scan_child_directory(&path, depth + 1, count, truncated, skipped_directories);
            if !children.is_empty() {
                entries.push(WorkspaceEntry {
                    name,
                    path,
                    is_dir: true,
                    children,
                });
            }
        } else if metadata.is_file() && is_markdown_path(&path) {
            entries.push(WorkspaceEntry {
                name,
                path,
                is_dir: false,
                children: Vec::new(),
            });
        }
    }

    entries.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(entries)
}

fn scan_child_directory(
    directory: &Path,
    depth: usize,
    count: &mut usize,
    truncated: &mut bool,
    skipped_directories: &mut usize,
) -> Vec<WorkspaceEntry> {
    match scan_directory(directory, depth, count, truncated, skipped_directories) {
        Ok(children) => children,
        Err(_) => {
            *skipped_directories = (*skipped_directories).saturating_add(1);
            Vec::new()
        }
    }
}

fn should_ignore(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name.to_ascii_lowercase().as_str(),
            "node_modules" | "target" | "dist" | "vendor"
        )
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "mdown" | "mkd" | "txt"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_markdown_recursively_and_ignores_build_directories() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("README.md"), "# root").unwrap();
        fs::write(directory.path().join("image.png"), "not markdown").unwrap();
        fs::create_dir(directory.path().join("notes")).unwrap();
        fs::write(directory.path().join("notes").join("two.MD"), "# nested").unwrap();
        fs::create_dir(directory.path().join("target")).unwrap();
        fs::write(
            directory.path().join("target").join("ignored.md"),
            "ignored",
        )
        .unwrap();

        let workspace = Workspace::open(directory.path().to_path_buf()).unwrap();
        assert_eq!(workspace.entries.len(), 2);
        assert!(workspace.entries[0].is_dir);
        assert_eq!(workspace.entries[0].children[0].name, "two.MD");
        assert_eq!(workspace.entries[1].name, "README.md");
        assert!(!workspace.truncated);
        assert_eq!(workspace.skipped_directories, 0);
    }

    #[test]
    fn omits_directories_without_supported_documents() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("empty")).unwrap();
        fs::create_dir(directory.path().join("images")).unwrap();
        fs::write(directory.path().join("images").join("one.png"), "image").unwrap();

        let workspace = Workspace::open(directory.path().to_path_buf()).unwrap();
        assert!(workspace.entries.is_empty());
    }

    #[test]
    fn unreadable_or_disappearing_child_directories_do_not_abort_the_scan() {
        let directory = tempfile::tempdir().unwrap();
        let mut count = 0;
        let mut truncated = false;
        let mut skipped = 0;
        let entries = scan_child_directory(
            &directory.path().join("missing"),
            1,
            &mut count,
            &mut truncated,
            &mut skipped,
        );
        assert!(entries.is_empty());
        assert_eq!(skipped, 1);
    }

    #[test]
    fn unsupported_files_and_empty_directories_consume_the_scan_budget() {
        for empty_directories in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            for index in 0..3 {
                let path = directory.path().join(format!("entry-{index}.png"));
                if empty_directories {
                    fs::create_dir(path).unwrap();
                } else {
                    fs::write(path, "image").unwrap();
                }
            }
            let mut count = MAX_ENTRIES - 2;
            let mut truncated = false;
            let mut skipped = 0;
            let entries = scan_directory(
                directory.path(),
                0,
                &mut count,
                &mut truncated,
                &mut skipped,
            )
            .unwrap();
            assert!(entries.is_empty());
            assert_eq!(count, MAX_ENTRIES);
            assert!(truncated);
            assert_eq!(skipped, 0);
        }
    }

    #[test]
    fn a_subtree_without_markdown_consumes_the_shared_scan_budget() {
        let directory = tempfile::tempdir().unwrap();
        let images = directory.path().join("images");
        fs::create_dir(&images).unwrap();
        for index in 0..3 {
            fs::write(images.join(format!("image-{index}.png")), "image").unwrap();
        }
        let mut count = MAX_ENTRIES - 3;
        let mut truncated = false;
        let mut skipped = 0;
        let entries = scan_directory(
            directory.path(),
            0,
            &mut count,
            &mut truncated,
            &mut skipped,
        )
        .unwrap();
        assert!(entries.is_empty());
        assert_eq!(count, MAX_ENTRIES);
        assert!(truncated);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn ancestors_do_not_exceed_the_scan_budget_after_the_last_document() {
        let directory = tempfile::tempdir().unwrap();
        let notes = directory.path().join("notes");
        fs::create_dir(&notes).unwrap();
        fs::write(notes.join("last.md"), "# last").unwrap();
        for remaining in [1, 2] {
            let mut count = MAX_ENTRIES - remaining;
            let mut truncated = false;
            let mut skipped = 0;
            let entries = scan_directory(
                directory.path(),
                0,
                &mut count,
                &mut truncated,
                &mut skipped,
            )
            .unwrap();
            assert_eq!(count, MAX_ENTRIES);
            assert_eq!(truncated, remaining == 1);
            assert_eq!(entries.len(), usize::from(remaining == 2));
            if remaining == 2 {
                assert_eq!(entries[0].children[0].name, "last.md");
            }
        }
    }
}
