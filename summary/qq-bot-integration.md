# QQ Bot Integration Summary

## Scope

Kokoro integrates the official QQ Bot API v2 as the `qq` Bot platform. The first implementation supports C2C messages and group mention messages with passive text replies.

## Backend Structure

- `src-tauri/src/qqbot/config.rs` defines persistent QQ credentials, access policy, and character binding.
- `src-tauri/src/qqbot/protocol.rs` is the functional core for dispatch parsing, mention cleanup, allowlist decisions, conversation keys, and passive reply bodies.
- `src-tauri/src/qqbot/runtime.rs` is the imperative shell for Access Token caching, Gateway discovery, IDENTIFY/RESUME, heartbeat ACK monitoring, reconnect, deduplication, bounded per-conversation sessions, AI dispatch, and OpenAPI replies.
- `src-tauri/src/commands/bot.rs` owns lifecycle integration, status, persistence normalization, and the shared text-reply wrapper.
- `AIOrchestrator::fork_with_isolated_history` snapshots persona/language settings while sharing model and long-term-memory services. It isolates recent history and database conversation state for each QQ peer without changing the desktop hot-reload conversation pointer.

## Protocol Contracts

- Token: `POST https://bots.qq.com/app/getAppAccessToken`.
- Gateway: `GET https://api.sgroup.qq.com/gateway` with `Authorization: QQBot <token>`.
- Intent: bit 25 (`GROUP_AND_C2C`).
- Events: `C2C_MESSAGE_CREATE` and `GROUP_AT_MESSAGE_CREATE`.
- Replies: `/v2/users/{openid}/messages` or `/v2/groups/{group_openid}/messages`, using the inbound `msg_id`.
- Conversation keys: `qq:c2c:{user_openid}` and `qq:group:{group_openid}:{member_openid}`.

## Configuration

The UI accepts AppID/AppSecret directly or through `QQBOT_APP_ID` and `QQBOT_APP_SECRET`. Empty allowlists reject all QQ traffic. C2C requires an allowed user; groups require an allowed group and may additionally restrict member OpenIDs.

QQ's token endpoint may return HTTP 200 for rejected credentials. Kokoro therefore validates the response body as a success-or-business-error contract, reports the QQ error code/message and TraceId, and accepts numeric or string `expires_in` values.

For first-time C2C access, Kokoro opens an authorization dialog instead of requiring users to copy OpenIDs from logs. The pending message remains in memory for up to 120 seconds. Allowing the account atomically adds it to the QQ allowlist and continues that first message; rejecting suppresses repeat prompts until the app restarts.

The same flow applies to group mentions. The dialog shows the group and triggering member OpenIDs. Approval adds the group to the allowlist and resumes the first message; the member is added only when member-level restrictions were already enabled.

## Verification

- Frontend Vitest: 84 tests passed.
- Frontend production build: passed.
- Rust test target compilation: passed.
- Rust Clippy with `-D warnings`: passed.
- Locale JSON parsing and `git diff --check`: passed.
- Independent final code review: zero findings.
- Rust test execution is blocked before assertions by the repository's existing ONNX Runtime DLL mismatch (`STATUS_ENTRYPOINT_NOT_FOUND`).
