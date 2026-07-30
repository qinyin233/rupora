use std::time::Duration;

use semver::Version;
use serde::Deserialize;

pub const RELEASES_URL: &str = "https://github.com/qinyin233/rupora/releases";
const RELEASES_API: &str = "https://api.github.com/repos/qinyin233/rupora/releases?per_page=20";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: Version,
    pub page_url: String,
    pub notes: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Current { latest: Version },
    Available(UpdateInfo),
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
    status_from_release(&current, release)
}

fn status_from_release(current: &Version, release: GitHubRelease) -> Result<UpdateStatus, String> {
    if release.draft {
        return Err("latest release is still a draft".to_owned());
    }
    let latest = parse_version(&release.tag_name)?;
    if latest > *current && (!release.prerelease || !current.pre.is_empty()) {
        Ok(UpdateStatus::Available(UpdateInfo {
            version: latest,
            page_url: release.html_url,
            notes: release
                .body
                .unwrap_or_default()
                .chars()
                .take(4_000)
                .collect(),
        }))
    } else {
        Ok(UpdateStatus::Current { latest })
    }
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
        }
    }

    #[test]
    fn recognizes_a_new_stable_release() {
        let status =
            status_from_release(&Version::parse("2.0.0").unwrap(), release("v2.1.0", false))
                .unwrap();

        assert!(matches!(status, UpdateStatus::Available(_)));
    }

    #[test]
    fn does_not_offer_prereleases_to_stable_users() {
        let status = status_from_release(
            &Version::parse("2.0.0").unwrap(),
            release("v2.1.0-beta.1", true),
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
        )
        .unwrap();

        assert!(matches!(status, UpdateStatus::Available(_)));
    }

    #[test]
    fn accepts_release_metadata_without_notes() {
        let mut release = release("v2.1.0", false);
        release.body = None;

        let status = status_from_release(&Version::parse("2.0.0").unwrap(), release).unwrap();

        let UpdateStatus::Available(update) = status else {
            panic!("expected an update");
        };
        assert!(update.notes.is_empty());
    }
}
