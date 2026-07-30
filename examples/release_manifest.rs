use std::{
    env, fs,
    path::{Path, PathBuf},
};

use rupora::updater::{
    SignedUpdateManifest, UpdateArtifact, UpdatePayload, decode_signing_key, decode_verifying_key,
    derive_verifying_key, sign_manifest,
};
use sha2::{Digest as _, Sha256};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let [version, target, artifact_directory, output] =
        env::args().skip(1).collect::<Vec<_>>().try_into().map_err(
            |_: Vec<String>| {
                "usage: release_manifest <version> <target> <artifact-directory> <output>"
            },
        )?;
    let signing_key = env::var("RUPORA_UPDATE_SIGNING_KEY")
        .map_err(|_| "RUPORA_UPDATE_SIGNING_KEY is not configured".to_owned())
        .and_then(|value| decode_signing_key(&value))?;
    let expected_public_key = env::var("RUPORA_UPDATE_PUBLIC_KEY")
        .map_err(|_| "RUPORA_UPDATE_PUBLIC_KEY is not configured".to_owned())
        .and_then(|value| decode_verifying_key(&value))?;
    if derive_verifying_key(&signing_key) != expected_public_key {
        return Err("update signing and verification keys do not match".into());
    }

    let version = version.trim_start_matches(['v', 'V']).to_owned();
    let tag = format!("v{version}");
    let artifact_directory = PathBuf::from(artifact_directory);
    let artifacts = collect_artifacts(&artifact_directory, &tag)?;
    let manifest = sign_manifest(
        UpdatePayload {
            schema: 1,
            version,
            target,
            artifacts,
        },
        &signing_key,
    )?;
    write_manifest(Path::new(&output), &manifest)?;
    Ok(())
}

fn collect_artifacts(directory: &Path, tag: &str) -> Result<Vec<UpdateArtifact>, String> {
    let mut paths = fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_package(path))
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    if paths.is_empty() {
        return Err(format!("no packages found in {}", directory.display()));
    }

    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("artifact name is not UTF-8: {}", path.display()))?
                .to_owned();
            if !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            {
                return Err(format!("artifact name is not URL-safe: {name}"));
            }
            let bytes = fs::read(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            Ok(UpdateArtifact {
                url: format!("https://github.com/qinyin233/rupora/releases/download/{tag}/{name}"),
                name,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                size: bytes.len() as u64,
            })
        })
        .collect()
}

fn is_package(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "exe" | "msi" | "dmg" | "deb" | "appimage"
            )
        })
}

fn write_manifest(path: &Path, manifest: &SignedUpdateManifest) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("cannot serialize signed manifest: {error}"))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("cannot commit {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_only_sorted_package_artifacts() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("z.exe"), b"windows").unwrap();
        fs::write(directory.path().join("a.msi"), b"installer").unwrap();
        fs::write(directory.path().join("rupora.cdx.json"), b"sbom").unwrap();

        let artifacts = collect_artifacts(directory.path(), "v2.1.0").unwrap();

        assert_eq!(
            artifacts
                .iter()
                .map(|artifact| artifact.name.as_str())
                .collect::<Vec<_>>(),
            ["a.msi", "z.exe"]
        );
        assert!(
            artifacts[0]
                .url
                .ends_with("/releases/download/v2.1.0/a.msi")
        );
        assert_eq!(artifacts[0].size, 9);
    }
}
