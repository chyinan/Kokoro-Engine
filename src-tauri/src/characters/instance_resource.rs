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
