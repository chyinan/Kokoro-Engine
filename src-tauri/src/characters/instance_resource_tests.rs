// pattern: Functional Core

use super::instance_resource::parse_instance_avatar_request;

#[test]
fn parses_direct_custom_scheme_avatar_request() {
    assert_eq!(
        parse_instance_avatar_request(Some("instance-1"), "/avatar.png"),
        Some(("instance-1".to_string(), "avatar.png".to_string()))
    );
}

#[test]
fn parses_windows_webview2_avatar_request() {
    assert_eq!(
        parse_instance_avatar_request(
            Some("character-instance-resource.localhost"),
            "/instance-1/avatar.png",
        ),
        Some(("instance-1".to_string(), "avatar.png".to_string()))
    );
}

#[test]
fn rejects_traversal_and_unknown_hosts() {
    assert_eq!(
        parse_instance_avatar_request(
            Some("character-instance-resource.localhost"),
            "/instance-1/%2e%2e/secret.png",
        ),
        None
    );
    assert_eq!(
        parse_instance_avatar_request(Some("other.localhost"), "/avatar.png"),
        None
    );
}
