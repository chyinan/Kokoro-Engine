// pattern: Functional Core

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

pub const REGISTRY_SCHEMA_VERSION: u32 = 1;
pub const REGISTRY_VERSION: u32 = 1;
pub const OFFICIAL_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/v1/index.json";
pub const OFFICIAL_PACKAGE_BASE_URL: &str =
    "https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/packages";
pub const OFFICIAL_REGISTRY_IDENTITY: &str = "github.com/chyinan/Kokoro-Engine/registry-v1";
/// URI sentinel for entries fetched from the canonical endpoint whose
/// self-asserted trust metadata was incomplete or inconsistent.
pub const OFFICIAL_REGISTRY_METADATA_UNVERIFIED_SOURCE: &str =
    "https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/v1/index.json#metadata-unverified";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistryManifestError {
    #[error("invalid registry JSON: {0}")]
    InvalidJson(String),
    #[error("unsupported registry schema version {0}")]
    UnsupportedSchema(u32),
    #[error("unsupported registry version {0}")]
    UnsupportedRegistryVersion(u32),
    #[error("registry index must contain at least one entry")]
    EmptyEntries,
    #[error("duplicate registry entry `{content_type}:{id}`")]
    DuplicateId { content_type: String, id: String },
    #[error("missing registry field `{0}`")]
    MissingField(&'static str),
    #[error("invalid registry content type `{0}`")]
    InvalidContentType(String),
    #[error("invalid registry id `{0}`")]
    InvalidId(String),
    #[error("invalid semantic version `{value}`: {reason}")]
    InvalidVersion { value: String, reason: String },
    #[error("invalid engine version range `{value}`: {reason}")]
    InvalidEngineVersion { value: String, reason: String },
    #[error("invalid registry URL `{0}`")]
    InvalidUrl(String),
    #[error("registry URL basename must be `{expected}`, got `{actual}`")]
    ArchiveNameMismatch { expected: String, actual: String },
    #[error("invalid archive size `{0}`")]
    InvalidArchiveSize(u64),
    #[error("invalid SHA-256 checksum `{0}`")]
    InvalidChecksum(String),
    #[error("invalid trust label `{0}`")]
    InvalidTrust(String),
    #[error("official trust requires the canonical registry endpoint and identity")]
    InvalidOfficialTrust,
    #[error("non-official entries cannot claim the official registry identity")]
    NonOfficialIdentity,
    #[error("invalid permission `{0}`")]
    InvalidPermission(String),
    #[error("invalid recommendation `{0}`")]
    InvalidRecommendation(String),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegistryIndex {
    pub schema_version: u32,
    pub registry_version: u32,
    #[serde(default)]
    pub generated_at: Option<String>,
    pub entries: Vec<RegistryEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegistryEntry {
    pub content_type: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    #[serde(default)]
    pub preview: Vec<String>,
    pub engine_version: String,
    pub download_url: String,
    pub archive_size: u64,
    pub sha256: String,
    pub trust: String,
    pub trust_source: String,
    #[serde(default)]
    pub registry_identity: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    pub recommendations: RegistryRecommendations,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegistryRecommendations {
    pub vision: bool,
    pub memory: bool,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    #[serde(default)]
    pub bot_platforms: Vec<String>,
}

impl RegistryIndex {
    pub fn from_json(json: &str) -> Result<Self, RegistryManifestError> {
        let index: Self = serde_json::from_str(json)
            .map_err(|error| RegistryManifestError::InvalidJson(error.to_string()))?;
        index.validate()?;
        Ok(index)
    }

    pub fn validate(&self) -> Result<(), RegistryManifestError> {
        if self.schema_version != REGISTRY_SCHEMA_VERSION {
            return Err(RegistryManifestError::UnsupportedSchema(
                self.schema_version,
            ));
        }
        if self.registry_version != REGISTRY_VERSION {
            return Err(RegistryManifestError::UnsupportedRegistryVersion(
                self.registry_version,
            ));
        }
        if self.entries.is_empty() {
            return Err(RegistryManifestError::EmptyEntries);
        }
        let mut seen = std::collections::HashSet::new();
        for entry in &self.entries {
            entry.validate()?;
            let key = format!("{}:{}", entry.content_type, entry.id);
            if !seen.insert(key) {
                return Err(RegistryManifestError::DuplicateId {
                    content_type: entry.content_type.clone(),
                    id: entry.id.clone(),
                });
            }
        }
        Ok(())
    }
}

impl RegistryEntry {
    pub fn validate(&self) -> Result<(), RegistryManifestError> {
        for (field, value) in [
            ("content_type", self.content_type.as_str()),
            ("id", self.id.as_str()),
            ("name", self.name.as_str()),
            ("version", self.version.as_str()),
            ("author", self.author.as_str()),
            ("description", self.description.as_str()),
            ("engine_version", self.engine_version.as_str()),
            ("download_url", self.download_url.as_str()),
            ("sha256", self.sha256.as_str()),
            ("trust", self.trust.as_str()),
            ("trust_source", self.trust_source.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(RegistryManifestError::MissingField(field));
            }
        }
        if !matches!(self.content_type.as_str(), "character" | "mod") {
            return Err(RegistryManifestError::InvalidContentType(
                self.content_type.clone(),
            ));
        }
        if !is_valid_id(&self.id) {
            return Err(RegistryManifestError::InvalidId(self.id.clone()));
        }
        if self
            .preview
            .iter()
            .any(|preview| !is_valid_preview_reference(preview))
        {
            let invalid = self
                .preview
                .iter()
                .find(|preview| !is_valid_preview_reference(preview))
                .cloned()
                .unwrap_or_default();
            return Err(RegistryManifestError::InvalidUrl(invalid));
        }
        Version::parse(&self.version).map_err(|error| RegistryManifestError::InvalidVersion {
            value: self.version.clone(),
            reason: error.to_string(),
        })?;
        VersionReq::parse(&self.engine_version).map_err(|error| {
            RegistryManifestError::InvalidEngineVersion {
                value: self.engine_version.clone(),
                reason: error.to_string(),
            }
        })?;
        let archive_name = format!("{}-{}.zip", self.id, self.version);
        let parsed = reqwest::Url::parse(&self.download_url)
            .map_err(|_| RegistryManifestError::InvalidUrl(self.download_url.clone()))?;
        if parsed.scheme() != "https" || parsed.username() != "" || parsed.password().is_some() {
            return Err(RegistryManifestError::InvalidUrl(self.download_url.clone()));
        }
        let actual_name = parsed
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .unwrap_or_default();
        if actual_name != archive_name {
            return Err(RegistryManifestError::ArchiveNameMismatch {
                expected: archive_name,
                actual: actual_name.to_string(),
            });
        }
        if self.trust == "official" && !is_official_package_url(&parsed, &archive_name) {
            return Err(RegistryManifestError::InvalidOfficialTrust);
        }
        if self.archive_size == 0 {
            return Err(RegistryManifestError::InvalidArchiveSize(self.archive_size));
        }
        if !is_sha256(&self.sha256) {
            return Err(RegistryManifestError::InvalidChecksum(self.sha256.clone()));
        }
        let trust_source_url = reqwest::Url::parse(&self.trust_source)
            .map_err(|_| RegistryManifestError::InvalidUrl(self.trust_source.clone()))?;
        if trust_source_url.scheme() != "https"
            || !trust_source_url.username().is_empty()
            || trust_source_url.password().is_some()
        {
            return Err(RegistryManifestError::InvalidUrl(self.trust_source.clone()));
        }
        let source = normalize_trust_source(&self.trust_source);
        if !matches!(self.trust.as_str(), "official" | "community" | "unverified") {
            return Err(RegistryManifestError::InvalidTrust(self.trust.clone()));
        }
        if self.trust_source == OFFICIAL_REGISTRY_URL && self.trust != "official" {
            return Err(RegistryManifestError::InvalidOfficialTrust);
        }
        if self.trust == "official"
            && (source.trust != "official"
                || self.registry_identity.as_deref() != Some(OFFICIAL_REGISTRY_IDENTITY))
        {
            return Err(RegistryManifestError::InvalidOfficialTrust);
        }
        if self.trust != "official" && self.registry_identity.is_some() {
            return Err(RegistryManifestError::NonOfficialIdentity);
        }
        let mut seen_permissions = HashSet::new();
        for permission in &self.permissions {
            if !is_allowed_permission(permission) {
                return Err(RegistryManifestError::InvalidPermission(permission.clone()));
            }
            if !seen_permissions.insert(permission) {
                return Err(RegistryManifestError::InvalidPermission(permission.clone()));
            }
        }
        self.recommendations.validate()?;
        Ok(())
    }
}

impl RegistryRecommendations {
    fn validate(&self) -> Result<(), RegistryManifestError> {
        for values in [&self.mcp_servers, &self.bot_platforms] {
            let mut seen = HashSet::new();
            for value in values {
                if !is_valid_identifier(value) || !seen.insert(value) {
                    return Err(RegistryManifestError::InvalidRecommendation(value.clone()));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustSource {
    pub trust: &'static str,
    pub trust_source: String,
    pub registry_identity: Option<String>,
}

pub fn normalize_trust_source(source: &str) -> TrustSource {
    if source == OFFICIAL_REGISTRY_URL {
        TrustSource {
            trust: "official",
            trust_source: source.to_string(),
            registry_identity: Some(OFFICIAL_REGISTRY_IDENTITY.to_string()),
        }
    } else {
        TrustSource {
            trust: "community",
            trust_source: source.to_string(),
            registry_identity: None,
        }
    }
}

fn is_valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_allowed_permission(value: &str) -> bool {
    matches!(
        value,
        "tts"
            | "vision"
            | "memory"
            | "mcp"
            | "bot"
            | "system.info"
            | "system.notifications"
            | "filesystem.read"
            | "clipboard.read"
    )
}

fn is_valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_valid_preview_reference(value: &str) -> bool {
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_whitespace) {
        return false;
    }
    if value.starts_with("https://") {
        let Ok(parsed) = reqwest::Url::parse(value) else {
            return false;
        };
        return parsed.scheme() == "https"
            && parsed.host_str().is_some_and(|host| !host.is_empty())
            && parsed.username().is_empty()
            && parsed.password().is_none();
    }
    if value.starts_with('/')
        || value.starts_with("//")
        || value.contains('\\')
        || value.contains(':')
    {
        return false;
    }
    value
        .split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn is_official_package_url(parsed: &reqwest::Url, archive_name: &str) -> bool {
    let Ok(base) = reqwest::Url::parse(OFFICIAL_PACKAGE_BASE_URL) else {
        return false;
    };
    parsed.scheme() == base.scheme()
        && parsed.host_str() == base.host_str()
        && parsed.port() == base.port()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed.path() == format!("{}/{}", base.path().trim_end_matches('/'), archive_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_entry() -> RegistryEntry {
        RegistryEntry {
            content_type: "character".to_string(),
            id: "kokoro".to_string(),
            name: "Kokoro".to_string(),
            version: "1.0.0".to_string(),
            author: "Kokoro Engine".to_string(),
            description: "A companion".to_string(),
            preview: Vec::new(),
            engine_version: ">=0.3.1, <0.4.0".to_string(),
            download_url: format!("{OFFICIAL_PACKAGE_BASE_URL}/kokoro-1.0.0.zip"),
            archive_size: 42,
            sha256: "a".repeat(64),
            trust: "official".to_string(),
            trust_source: OFFICIAL_REGISTRY_URL.to_string(),
            registry_identity: Some(OFFICIAL_REGISTRY_IDENTITY.to_string()),
            permissions: Vec::new(),
            recommendations: RegistryRecommendations {
                vision: false,
                memory: true,
                mcp_servers: Vec::new(),
                bot_platforms: Vec::new(),
            },
        }
    }

    #[test]
    fn validates_content_type_compatibility_and_archive_metadata() {
        let entry = valid_entry();
        assert!(entry.validate().is_ok());
        let mut mod_entry = entry.clone();
        mod_entry.content_type = "mod".to_string();
        assert!(mod_entry.validate().is_ok());
        let mut incompatible = entry.clone();
        incompatible.engine_version = "not-a-range".to_string();
        assert!(matches!(
            incompatible.validate(),
            Err(RegistryManifestError::InvalidEngineVersion { .. })
        ));
    }

    #[test]
    fn validates_https_url_basename_checksum_and_size() {
        let mut entry = valid_entry();
        entry.download_url = "http://example.test/kokoro-1.0.0.zip".to_string();
        assert!(matches!(
            entry.validate(),
            Err(RegistryManifestError::InvalidUrl(_))
        ));
        let mut entry = valid_entry();
        entry.download_url = format!("{OFFICIAL_PACKAGE_BASE_URL}/other.zip");
        assert!(matches!(
            entry.validate(),
            Err(RegistryManifestError::ArchiveNameMismatch { .. })
        ));
        let mut entry = valid_entry();
        entry.sha256 = "nope".to_string();
        assert!(matches!(
            entry.validate(),
            Err(RegistryManifestError::InvalidChecksum(_))
        ));
        let mut entry = valid_entry();
        entry.archive_size = 0;
        assert!(matches!(
            entry.validate(),
            Err(RegistryManifestError::InvalidArchiveSize(0))
        ));
    }

    #[test]
    fn validates_preview_references_without_allowing_active_content() {
        let mut entry = valid_entry();
        entry.preview = vec!["https://cdn.example.test/kokoro.webp".to_string()];
        assert!(entry.validate().is_ok());
        entry.preview = vec!["assets/kokoro.webp".to_string()];
        assert!(entry.validate().is_ok());
        entry.preview = vec!["javascript:alert(1)".to_string()];
        assert!(matches!(
            entry.validate(),
            Err(RegistryManifestError::InvalidUrl(_))
        ));
        entry.preview = vec!["../outside.webp".to_string()];
        assert!(matches!(
            entry.validate(),
            Err(RegistryManifestError::InvalidUrl(_))
        ));
    }

    #[test]
    fn validates_trust_permissions_and_recommendations() {
        let mut entry = valid_entry();
        entry.permissions = vec!["exec.shell".to_string()];
        assert!(matches!(
            entry.validate(),
            Err(RegistryManifestError::InvalidPermission(_))
        ));
        let mut entry = valid_entry();
        entry.recommendations.mcp_servers = vec!["not safe/identifier".to_string()];
        assert!(matches!(
            entry.validate(),
            Err(RegistryManifestError::InvalidRecommendation(_))
        ));
        let mut entry = valid_entry();
        entry.trust_source = "https://mirror.example.test/index.json".to_string();
        assert!(matches!(
            entry.validate(),
            Err(RegistryManifestError::InvalidOfficialTrust)
        ));
        let normalized = normalize_trust_source("https://mirror.example.test/index.json");
        assert_eq!(normalized.trust, "community");
        assert_eq!(normalized.registry_identity, None);
    }

    #[test]
    fn shared_unverified_trust_source_sentinel_remains_non_official_and_schema_safe() {
        let mut entry = valid_entry();
        entry.trust = "community".to_string();
        entry.trust_source = OFFICIAL_REGISTRY_METADATA_UNVERIFIED_SOURCE.to_string();
        entry.registry_identity = None;

        assert!(entry.validate().is_ok());
        let normalized = normalize_trust_source(OFFICIAL_REGISTRY_METADATA_UNVERIFIED_SOURCE);
        assert_eq!(normalized.trust, "community");
        assert_eq!(normalized.registry_identity, None);
    }

    #[test]
    fn non_official_entries_cannot_reuse_the_canonical_trust_source() {
        let mut entry = valid_entry();
        entry.trust = "community".to_string();
        entry.trust_source = OFFICIAL_REGISTRY_URL.to_string();
        entry.registry_identity = None;

        assert!(matches!(
            entry.validate(),
            Err(RegistryManifestError::InvalidOfficialTrust)
        ));
    }

    #[test]
    fn rejects_duplicate_ids_per_content_type_and_accepts_cross_type_ids() {
        let entry = valid_entry();
        let duplicate = RegistryIndex {
            schema_version: 1,
            registry_version: 1,
            generated_at: None,
            entries: vec![entry.clone(), entry],
        };
        assert!(matches!(
            duplicate.validate(),
            Err(RegistryManifestError::DuplicateId { .. })
        ));
        let mut cross_type = valid_entry();
        cross_type.content_type = "mod".to_string();
        let index = RegistryIndex {
            schema_version: 1,
            registry_version: 1,
            generated_at: None,
            entries: vec![valid_entry(), cross_type],
        };
        assert!(index.validate().is_ok());
    }

    #[test]
    fn parses_json_and_rejects_unknown_versions() {
        let entry = valid_entry();
        let value = json!({
            "schema_version": 1,
            "registry_version": 1,
            "entries": [entry]
        });
        assert!(RegistryIndex::from_json(&value.to_string()).is_ok());
        let mut value = value;
        value["schema_version"] = json!(2);
        assert!(matches!(
            RegistryIndex::from_json(&value.to_string()),
            Err(RegistryManifestError::UnsupportedSchema(2))
        ));
    }

    #[test]
    fn requires_lowercase_checksums_and_unique_permission_recommendation_lists() {
        let mut uppercase = valid_entry();
        uppercase.sha256 = "A".repeat(64);
        assert!(matches!(
            uppercase.validate(),
            Err(RegistryManifestError::InvalidChecksum(_))
        ));

        let mut duplicate_permission = valid_entry();
        duplicate_permission.permissions = vec!["tts".to_string(), "tts".to_string()];
        assert!(matches!(
            duplicate_permission.validate(),
            Err(RegistryManifestError::InvalidPermission(_))
        ));

        let mut duplicate_recommendation = valid_entry();
        duplicate_recommendation.recommendations.mcp_servers =
            vec!["calendar".to_string(), "calendar".to_string()];
        assert!(matches!(
            duplicate_recommendation.validate(),
            Err(RegistryManifestError::InvalidRecommendation(_))
        ));
    }
}
