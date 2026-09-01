// pattern: Functional Core

use thiserror::Error;

pub const PNG_SIGNATURE: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10];
pub const MAX_INSTANCE_AVATAR_BYTES: usize = 16 * 1024 * 1024;
const INSTANCE_AVATAR_SCHEME: &str = "character-instance-resource://";
const INSTANCE_AVATAR_SUFFIX: &str = "/avatar.png";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InstanceResourceError {
    #[error("invalid character instance resource id")]
    InvalidInstanceId,
    #[error("character avatar must be a PNG no larger than 16 MiB")]
    InvalidAvatar,
}

pub fn validate_instance_id(instance_id: &str) -> Result<(), InstanceResourceError> {
    if instance_id.is_empty()
        || instance_id.len() > 128
        || !instance_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        Err(InstanceResourceError::InvalidInstanceId)
    } else {
        Ok(())
    }
}

pub fn validate_avatar_bytes(bytes: &[u8]) -> Result<(), InstanceResourceError> {
    if bytes.starts_with(PNG_SIGNATURE) && bytes.len() <= MAX_INSTANCE_AVATAR_BYTES {
        Ok(())
    } else {
        Err(InstanceResourceError::InvalidAvatar)
    }
}

pub fn instance_avatar_reference(instance_id: &str) -> Result<String, InstanceResourceError> {
    validate_instance_id(instance_id)?;
    Ok(format!(
        "{INSTANCE_AVATAR_SCHEME}{instance_id}{INSTANCE_AVATAR_SUFFIX}"
    ))
}

pub fn parse_instance_avatar_reference(value: &str) -> Option<&str> {
    let instance_id = value
        .strip_prefix(INSTANCE_AVATAR_SCHEME)?
        .strip_suffix(INSTANCE_AVATAR_SUFFIX)?;
    validate_instance_id(instance_id).ok()?;
    Some(instance_id)
}

/// Parses both native custom-scheme and Windows WebView2 avatar requests.
///
/// Native WebKit keeps the instance id in the URI host. Wry's Windows
/// workaround maps `character-instance-resource://localhost/id/avatar.png`
/// to `http://character-instance-resource.localhost/id/avatar.png` and then
/// restores the `localhost` form before invoking this handler.
pub fn parse_instance_avatar_request(host: Option<&str>, path: &str) -> Option<(String, String)> {
    let host = host?;
    let clean_path = path.trim_start_matches('/');
    let (instance_id, relative) = if host
        .eq_ignore_ascii_case("character-instance-resource.localhost")
        || host.eq_ignore_ascii_case("localhost")
    {
        clean_path.split_once('/')?
    } else {
        if host.ends_with(".localhost") {
            return None;
        }
        (host, clean_path)
    };

    validate_instance_id(instance_id).ok()?;
    if relative != "avatar.png" || relative.contains("..") || relative.contains('\\') {
        return None;
    }
    Some((instance_id.to_string(), relative.to_string()))
}
