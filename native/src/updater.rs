use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const RELEASES_URL: &str = "https://github.com/qinyin233/rupora/releases";
const RELEASES_API: &str = "https://api.github.com/repos/qinyin233/rupora/releases?per_page=20";
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_UPDATE_ARTIFACTS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateArtifact {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePayload {
    pub schema: u32,
    pub version: String,
    pub target: String,
    pub artifacts: Vec<UpdateArtifact>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedUpdateManifest {
    pub payload: UpdatePayload,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: Version,
    pub page_url: String,
    pub notes: String,
    pub target: String,
    pub artifacts: Vec<UpdateArtifact>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Current { latest: Version },
    Available(UpdateInfo),
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

pub fn current_target() -> &'static str {
    env!("RUPORA_BUILD_TARGET")
}

pub fn manifest_asset_name(target: &str) -> String {
    format!("rupora-update-{target}.json")
}

pub fn sign_manifest(
    payload: UpdatePayload,
    signing_key: &[u8; 32],
) -> Result<SignedUpdateManifest, String> {
    validate_payload(&payload)?;
    let message = serde_json::to_vec(&payload)
        .map_err(|error| format!("cannot serialize update payload: {error}"))?;
    let signature = SigningKey::from_bytes(signing_key).sign(&message);
    Ok(SignedUpdateManifest {
        payload,
        signature: BASE64.encode(signature.to_bytes()),
    })
}

pub fn verify_manifest(
    manifest_json: &str,
    public_key: &[u8; 32],
    expected_target: &str,
    expected_version: &Version,
) -> Result<UpdatePayload, String> {
    if manifest_json.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("update manifest exceeds the size limit".to_owned());
    }
    let manifest: SignedUpdateManifest = serde_json::from_str(manifest_json)
        .map_err(|error| format!("invalid signed update manifest: {error}"))?;
    validate_payload(&manifest.payload)?;
    if manifest.payload.target != expected_target {
        return Err(format!(
            "update target mismatch: expected {expected_target}, got {}",
            manifest.payload.target
        ));
    }
    let version = parse_version(&manifest.payload.version)?;
    if version != *expected_version {
        return Err(format!(
            "update version mismatch: expected {expected_version}, got {version}"
        ));
    }

    let key = VerifyingKey::from_bytes(public_key)
        .map_err(|error| format!("invalid update verification key: {error}"))?;
    let signature_bytes = BASE64
        .decode(manifest.signature.as_bytes())
        .map_err(|error| format!("invalid update signature encoding: {error}"))?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|error| format!("invalid update signature: {error}"))?;
    let message = serde_json::to_vec(&manifest.payload)
        .map_err(|error| format!("cannot serialize update payload: {error}"))?;
    key.verify(&message, &signature)
        .map_err(|_| "update manifest signature verification failed".to_owned())?;
    Ok(manifest.payload)
}

pub fn verify_artifact(bytes: &[u8], artifact: &UpdateArtifact) -> Result<(), String> {
    if bytes.len() as u64 != artifact.size {
        return Err(format!(
            "artifact length mismatch: expected {}, got {}",
            artifact.size,
            bytes.len()
        ));
    }
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != artifact.sha256 {
        return Err(format!(
            "artifact checksum mismatch: expected {}, got {actual}",
            artifact.sha256
        ));
    }
    Ok(())
}

pub fn decode_signing_key(value: &str) -> Result<[u8; 32], String> {
    decode_key(value, "signing")
}

pub fn decode_verifying_key(value: &str) -> Result<[u8; 32], String> {
    decode_key(value, "verification")
}

pub fn derive_verifying_key(signing_key: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(signing_key)
        .verifying_key()
        .to_bytes()
}

pub fn check_for_update(current_version: &str) -> Result<UpdateStatus, String> {
    let current = parse_version(current_version)?;
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(12)))
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent
        .get(RELEASES_API)
        .header("User-Agent", concat!("RUPORA/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| format!("update request failed: {error}"))?;
    let releases: Vec<GitHubRelease> = response
        .body_mut()
        .read_json()
        .map_err(|error| format!("invalid update response: {error}"))?;
    let follows_prereleases = !current.pre.is_empty();
    let release = releases
        .into_iter()
        .filter(|release| !release.draft && (!release.prerelease || follows_prereleases))
        .filter_map(|release| {
            parse_version(&release.tag_name)
                .ok()
                .map(|version| (version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, release)| release)
        .ok_or_else(|| "no compatible releases were found".to_owned())?;
    let latest = parse_version(&release.tag_name)?;
    if latest <= current || (release.prerelease && current.pre.is_empty()) {
        return Ok(UpdateStatus::Current { latest });
    }

    let public_key = option_env!("RUPORA_UPDATE_PUBLIC_KEY")
        .ok_or_else(|| "update verification key is not configured in this build".to_owned())
        .and_then(decode_verifying_key)?;
    let target = current_target();
    let manifest_name = manifest_asset_name(target);
    let manifest_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == manifest_name)
        .ok_or_else(|| format!("release does not contain {manifest_name}"))?;
    let mut manifest_response = agent
        .get(&manifest_asset.browser_download_url)
        .header("User-Agent", concat!("RUPORA/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|error| format!("update manifest request failed: {error}"))?;
    let manifest_json = manifest_response
        .body_mut()
        .with_config()
        .limit(MAX_MANIFEST_BYTES + 1)
        .read_to_string()
        .map_err(|error| format!("cannot read update manifest: {error}"))?;
    let payload = verify_manifest(&manifest_json, &public_key, target, &latest)?;
    status_from_release(&current, release, Some(payload))
}

fn status_from_release(
    current: &Version,
    release: GitHubRelease,
    verified_payload: Option<UpdatePayload>,
) -> Result<UpdateStatus, String> {
    if release.draft {
        return Err("latest release is still a draft".to_owned());
    }
    let latest = parse_version(&release.tag_name)?;
    if latest > *current && (!release.prerelease || !current.pre.is_empty()) {
        let payload = verified_payload
            .ok_or_else(|| "new release has no verified update manifest".to_owned())?;
        Ok(UpdateStatus::Available(UpdateInfo {
            version: latest,
            page_url: release.html_url,
            notes: release
                .body
                .unwrap_or_default()
                .chars()
                .take(4_000)
                .collect(),
            target: payload.target,
            artifacts: payload.artifacts,
        }))
    } else {
        Ok(UpdateStatus::Current { latest })
    }
}

fn validate_payload(payload: &UpdatePayload) -> Result<(), String> {
    if payload.schema != 1 {
        return Err(format!(
            "unsupported update manifest schema {}",
            payload.schema
        ));
    }
    parse_version(&payload.version)?;
    if payload.target.is_empty()
        || payload.target.contains(['\r', '\n'])
        || payload.artifacts.is_empty()
        || payload.artifacts.len() > MAX_UPDATE_ARTIFACTS
    {
        return Err("update manifest has invalid target or artifact count".to_owned());
    }
    let mut previous_name = None;
    for artifact in &payload.artifacts {
        if artifact.name.is_empty()
            || artifact.name.contains(['/', '\\', '\r', '\n'])
            || !artifact.url.starts_with("https://")
            || artifact.sha256.len() != 64
            || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || artifact
                .sha256
                .bytes()
                .any(|byte| byte.is_ascii_uppercase())
            || artifact.size == 0
        {
            return Err(format!("invalid update artifact {}", artifact.name));
        }
        if previous_name.is_some_and(|previous| previous >= artifact.name.as_str()) {
            return Err("update artifacts must be sorted by unique name".to_owned());
        }
        previous_name = Some(artifact.name.as_str());
    }
    Ok(())
}

fn decode_key(value: &str, kind: &str) -> Result<[u8; 32], String> {
    let decoded = BASE64
        .decode(value.trim().as_bytes())
        .map_err(|error| format!("invalid update {kind} key encoding: {error}"))?;
    decoded
        .try_into()
        .map_err(|_| format!("update {kind} key must contain exactly 32 bytes"))
}

fn parse_version(value: &str) -> Result<Version, String> {
    Version::parse(value.trim().trim_start_matches(['v', 'V']))
        .map_err(|error| format!("invalid release version {value:?}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.to_owned(),
            html_url: format!("{RELEASES_URL}/tag/{tag}"),
            body: Some("release notes".to_owned()),
            draft: false,
            prerelease,
            assets: Vec::new(),
        }
    }

    fn signed_payload(version: &str) -> (UpdatePayload, [u8; 32]) {
        let signing_key = [7u8; 32];
        let bytes = b"native package";
        let artifact = UpdateArtifact {
            name: "rupora-test.zip".to_owned(),
            url: "https://example.test/rupora-test.zip".to_owned(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
            size: bytes.len() as u64,
        };
        (
            UpdatePayload {
                schema: 1,
                version: version.to_owned(),
                target: "test-target".to_owned(),
                artifacts: vec![artifact],
            },
            signing_key,
        )
    }

    fn verified_payload(version: &str) -> UpdatePayload {
        signed_payload(version).0
    }

    #[test]
    fn verifies_a_signed_manifest_and_artifact() {
        let (payload, secret) = signed_payload("2.1.0");
        let public = SigningKey::from_bytes(&secret).verifying_key().to_bytes();
        let manifest = sign_manifest(payload.clone(), &secret).unwrap();
        let json = serde_json::to_string(&manifest).unwrap();

        let verified = verify_manifest(
            &json,
            &public,
            "test-target",
            &Version::parse("2.1.0").unwrap(),
        )
        .unwrap();
        assert_eq!(verified, payload);
        verify_artifact(b"native package", &verified.artifacts[0]).unwrap();
    }

    #[test]
    fn rejects_manifest_tampering_wrong_targets_and_bad_artifacts() {
        let (payload, secret) = signed_payload("2.1.0");
        let public = SigningKey::from_bytes(&secret).verifying_key().to_bytes();
        let manifest = sign_manifest(payload, &secret).unwrap();
        let mut value = serde_json::to_value(manifest).unwrap();
        value["payload"]["artifacts"][0]["size"] = 1.into();
        let tampered = serde_json::to_string(&value).unwrap();

        assert!(
            verify_manifest(
                &tampered,
                &public,
                "test-target",
                &Version::parse("2.1.0").unwrap()
            )
            .unwrap_err()
            .contains("signature")
        );

        let (payload, secret) = signed_payload("2.1.0");
        let manifest = sign_manifest(payload, &secret).unwrap();
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(
            verify_manifest(
                &json,
                &public,
                "another-target",
                &Version::parse("2.1.0").unwrap()
            )
            .unwrap_err()
            .contains("target mismatch")
        );
        assert!(verify_artifact(b"changed", &manifest.payload.artifacts[0]).is_err());
    }

    #[test]
    fn recognizes_a_new_stable_release_with_verified_metadata() {
        let status = status_from_release(
            &Version::parse("2.0.0").unwrap(),
            release("v2.1.0", false),
            Some(verified_payload("2.1.0")),
        )
        .unwrap();

        assert!(matches!(status, UpdateStatus::Available(_)));
    }

    #[test]
    fn refuses_to_offer_an_unsigned_new_release() {
        let error = status_from_release(
            &Version::parse("2.0.0").unwrap(),
            release("v2.1.0", false),
            None,
        )
        .unwrap_err();
        assert!(error.contains("verified update manifest"));
    }

    #[test]
    fn does_not_offer_prereleases_to_stable_users() {
        let status = status_from_release(
            &Version::parse("2.0.0").unwrap(),
            release("v2.1.0-beta.1", true),
            None,
        )
        .unwrap();

        assert_eq!(
            status,
            UpdateStatus::Current {
                latest: Version::parse("2.1.0-beta.1").unwrap()
            }
        );
    }

    #[test]
    fn allows_prerelease_users_to_follow_the_prerelease_channel() {
        let status = status_from_release(
            &Version::parse("2.0.0-alpha.1").unwrap(),
            release("v2.0.0-beta.1", true),
            Some(verified_payload("2.0.0-beta.1")),
        )
        .unwrap();

        assert!(matches!(status, UpdateStatus::Available(_)));
    }

    #[test]
    fn accepts_release_metadata_without_notes() {
        let mut release = release("v2.1.0", false);
        release.body = None;

        let status = status_from_release(
            &Version::parse("2.0.0").unwrap(),
            release,
            Some(verified_payload("2.1.0")),
        )
        .unwrap();

        let UpdateStatus::Available(update) = status else {
            panic!("expected an update");
        };
        assert!(update.notes.is_empty());
    }
}
