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
}

impl Workspace {
    pub fn open(root: PathBuf) -> Result<Self, String> {
        if !root.is_dir() {
            return Err(format!("工作区不是目录：{}", root.display()));
        }
        let mut count = 0;
        let mut truncated = false;
        let entries = scan_directory(&root, 0, &mut count, &mut truncated)?;
        Ok(Self {
            root,
            entries,
            truncated,
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
            let children = scan_directory(&path, depth + 1, count, truncated)?;
            if !children.is_empty() {
                *count += 1;
                entries.push(WorkspaceEntry {
                    name,
                    path,
                    is_dir: true,
                    children,
                });
            }
        } else if metadata.is_file() && is_markdown_path(&path) {
            *count += 1;
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
}
