# AstrBot integration

The Kokoro Engine AstrBot adapter forwards AstrBot events to Kokoro's generic
authenticated webhook. Kokoro owns character selection, conversation history,
LLM generation, transcription, TTS, and generated media. AstrBot owns the
channel connection and displays the reply chain.

This integration is a one-way HTTP adapter. It does not turn Kokoro into an
AstrBot rendering client, and it does not add a second character or real-time
embodiment protocol.

The plugin package is in [`integrations/astrbot-kokoro`](../../integrations/astrbot-kokoro/README.md).
It targets AstrBot `>=4.5.0` and uses the documented `Star`, `AstrBotConfig`,
`AstrMessageEvent`, `Plain`, `Image`, and `Record` APIs.

## Quick setup

### 1. Configure and start Kokoro's webhook

In Kokoro, open **Settings > Bots > Webhook** and set:

| Setting | Same-host default | Notes |
| --- | --- | --- |
| Enabled | On | The HTTP route returns `404` while the Webhook platform is disabled. |
| Bind host | `127.0.0.1` | Use only for clients on the same host. Container clients need a reachable host/interface. |
| Port | `8787` | Change both Kokoro and AstrBot if this is changed. |
| Endpoint path | `/webhook/message` | The leading slash is required. |
| Bearer token | A new random secret | Recommended. Leave empty only when unauthenticated local access is intentional. |
| Default character | Your fallback character | Used only when the request and plugin send no character ID. |
| Send voice replies (TTS) | Off | Turn on after configuring a working TTS provider. |

The token may be entered in Kokoro's settings or supplied through
`KOKORO_WEBHOOK_TOKEN`. Kokoro resolves a non-empty saved value first, then
the environment variable. Restart Kokoro after changing the environment.

Start the Webhook runtime after saving the settings. The default local URL is:

```text
http://127.0.0.1:8787/webhook/message
```

### 2. Install and configure the AstrBot plugin

Install the directory containing `main.py`, `metadata.yaml`,
`_conf_schema.json`, and `LICENSE` through AstrBot's plugin manager or its
development plugin directory. Restart AstrBot, enable **Kokoro Engine**, and
fill in:

| Plugin field | Default | Guidance |
| --- | --- | --- |
| `endpoint` | `http://127.0.0.1:8787/webhook/message` | Kokoro's complete URL, including the path. |
| `bearer_token` | empty | Copy the token value only. The plugin adds `Authorization: Bearer`. |
| `character_id` | `kokoro` | Character sent on every request. Clear it to defer to Kokoro. |
| `conversation_strategy` | `character_session` | Stable per-character mapping for private/group chats. |
| `enable_text` | `true` | Forward text and return text. |
| `enable_images` | `true` | Forward images and return image replies. |
| `enable_audio` | `true` | Forward audio to Kokoro STT and return audio replies. |

The plugin does not have a separate voice-reply switch. Enable Kokoro's
Webhook **Send voice replies (TTS)** for output audio and keep the plugin's
`enable_audio` enabled so the returned `audio` component is delivered to the
channel.

### 3. Send a first message

Send a short text message in a configured AstrBot channel. The plugin sends a
JSON request, waits for Kokoro's JSON reply, and yields a text/image/audio
message chain. A successful response has HTTP `200` and a non-empty `reply`.

## Network layouts

The URL is resolved from AstrBot's network namespace. `127.0.0.1` means the
current machine on a host install and the current container on a container
install.

| Layout | Plugin endpoint | Required Kokoro networking |
| --- | --- | --- |
| AstrBot and Kokoro on one host | `http://127.0.0.1:8787/webhook/message` | Keep Kokoro bound to `127.0.0.1` unless another local interface is needed. |
| AstrBot Docker container, Kokoro on host | `http://host.docker.internal:8787/webhook/message` | Docker Desktop supplies this name. On Linux, add `host.docker.internal:host-gateway` or use a host-interface address. The host service must be reachable from the container. |
| AstrBot and Kokoro on one Docker network | `http://kokoro:8787/webhook/message` | Replace `kokoro` with the Compose service name. Bind Kokoro to `0.0.0.0` inside its container and keep the network private. |
| Kokoro Docker container, AstrBot on host | `http://127.0.0.1:8787/webhook/message` | Publish the port to loopback, for example `127.0.0.1:8787:8787`. |

Binding Kokoro to `0.0.0.0` makes the route reachable on every container or
host interface. Restrict the port with the Docker network and host firewall,
and keep Bearer authentication enabled. Don't expose this endpoint directly
to the public internet.

## Character selection and conversation identity

Kokoro resolves a character independently for every request:

1. A non-empty request `character_id`.
2. Kokoro's Webhook default `character_id`.
3. Kokoro's active character.
4. `default` if all three values are empty or unavailable.

The plugin's default is `kokoro`, so it intentionally takes precedence over
Kokoro's Webhook default. Set `character_id` to `pico`, `seren`, or a custom
character ID to route requests to that character. Clear the plugin field when
the Kokoro-side default/active selection should control the request.

Character IDs are references, not arbitrary prompt text. Kokoro resolves the
selected character before it chooses the persisted conversation, and scopes
the external identity to that character. This prevents the same AstrBot user
from mixing histories after switching characters.

### `character_session` (default)

The plugin sends these canonical fields:

- Private chat: `conversation_type: "private"`, `user_id` set to the sender,
  and `conversation_id` set to the sender when a session ID is available.
- Group chat: `conversation_type: "group"`, `conversation_id` set to the
  group ID, and `user_id` set to the sender.
- `source`: AstrBot's platform ID/name, used as a fallback identity.

Kokoro maps private identities as `private:<user_id>` (falling back to
`conversation_id` and `source`) and group identities as
`group:<conversation_id>` (falling back to `source` and `user_id`). It then
prefixes the key with the resolved character ID before creating or loading a
conversation. A group is shared by its members; a private chat is not.

### `platform_session`

Use this strategy when a platform's unified session origin should be the
conversation key. The plugin uses AstrBot's `unified_msg_origin` when the
event provides it, then falls back to the sender or group ID. Keep this value
stable. Changing strategies changes the conversation key and therefore starts
a separate history without deleting the old one.

## Media behavior

The plugin supports one or more `Plain` components, up to eight image reply
components, and one audio component per event/reply chain.

| Toggle | Incoming behavior | Outgoing behavior |
| --- | --- | --- |
| `enable_text` | Joins enabled plain text components into `text`. | Returns the `reply` string as `Plain`. |
| `enable_images` | Converts AstrBot `Image` components to `data:image/...;base64,...` values in `images`. | Converts each reply image's `data_base64` to an AstrBot `Image`. |
| `enable_audio` | Converts the first AstrBot `Record` to normalized WAV bytes in `audio_base64` with `audio_format: "wav"`. | Converts reply `audio.data_base64` to an AstrBot `Record`. |

If all components are disabled, the plugin does not make an HTTP request. An
audio request is transcribed by Kokoro before generation, so Kokoro must have
a working STT provider. Kokoro voice output requires a TTS provider and the
Kokoro Webhook voice-reply setting; the plugin's audio toggle must also be on.

## HTTP contract

The plugin sends `POST` with `Content-Type: application/json`. When a token is
configured, it also sends `Authorization: Bearer <token>`.

### Request sent by the plugin

```json
{
  "text": "Hello from AstrBot",
  "character_id": "kokoro",
  "conversation_type": "private",
  "conversation_id": "user-42",
  "user_id": "user-42",
  "source": "telegram"
}
```

Media fields are optional:

```json
{
  "text": "Describe this",
  "images": ["data:image/png;base64,..."],
  "audio_base64": "...",
  "audio_format": "ogg"
}
```

Kokoro also accepts `message` as a legacy alias for `text`, singular `image`,
`image_base64`, and `image_mime_type` for direct webhook clients. The AstrBot
plugin uses the canonical plural `images` form. URLs and data URLs are valid
image references; a raw image base64 value is wrapped with the declared MIME
type.

### Reply and status codes

```json
{
  "reply": "Hello from Kokoro",
  "translation": "你好，来自 Kokoro",
  "images": [
    {
      "prompt": "...",
      "mime_type": "image/png",
      "file_name": "image.png",
      "data_base64": "..."
    }
  ],
  "audio": {
    "mime_type": "audio/ogg",
    "file_name": "reply.ogg",
    "data_base64": "..."
  }
}
```

| Status | Meaning | Typical cause |
| ---: | --- | --- |
| `200` | `WebhookReply` JSON | Kokoro generated text and/or media. |
| `400` | `{"error":"..."}` | Invalid JSON/base64, or no text/media. |
| `401` | `{"error":"..."}` | Missing or incorrect Bearer token. |
| `404` | `{"error":"Not found"}` | Webhook disabled or path mismatch. |
| `500` | `{"error":"..."}` | LLM, STT, TTS, image, or other runtime failure. |

The plugin turns non-success responses into a short AstrBot text notice and
does not expose the configured token.

## Curl smoke tests

Run these from the same network namespace as AstrBot. They test the Kokoro
endpoint directly, which separates webhook configuration errors from plugin
loading errors.

```bash
export KOKORO_WEBHOOK_URL='http://127.0.0.1:8787/webhook/message'
export KOKORO_WEBHOOK_TOKEN='replace-with-your-token'

curl --fail-with-body --silent --show-error \
  -X POST "$KOKORO_WEBHOOK_URL" \
  -H "Authorization: Bearer $KOKORO_WEBHOOK_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"text":"Webhook smoke test","character_id":"kokoro","conversation_type":"private","user_id":"smoke-user","source":"astrbot-smoke"}'
```

The response should be HTTP `200` with a non-empty `reply` when an LLM is
configured. Repeat the request with the same `user_id` to check conversation
reuse. Use `"conversation_type":"group"` and a stable
`"conversation_id":"smoke-group"` to check group mapping.

For a media request, use a data URL to avoid a provider fetching a test URL:

```bash
curl --fail-with-body --silent --show-error \
  -X POST "$KOKORO_WEBHOOK_URL" \
  -H "Authorization: Bearer $KOKORO_WEBHOOK_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"text":"Describe this image","images":["data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="],"conversation_type":"group","conversation_id":"smoke-group","user_id":"smoke-user","source":"astrbot-smoke"}'
```

To check authentication without sending a valid token, expect `401`:

```bash
curl --silent --show-error -o /tmp/kokoro-auth-error.json -w '%{http_code}\n' \
  -X POST "$KOKORO_WEBHOOK_URL" \
  -H 'Authorization: Bearer deliberately-wrong-token' \
  -H 'Content-Type: application/json' \
  -d '{"text":"auth smoke test","conversation_type":"private","user_id":"smoke-user"}'
```

For audio, base64-encode a short local `sample.ogg`, set
`"audio_format":"ogg"`, and keep a working Kokoro STT configuration. The
endpoint accepts raw base64 and `data:audio/...;base64,...` values.

## Troubleshooting

### The plugin cannot connect

Check the endpoint from the AstrBot process. A host install can use
`127.0.0.1`; a container must use `host.docker.internal`, a host-gateway
address, or the Kokoro service name. Then confirm Kokoro's Webhook runtime is
started, the port is published, and the bind host accepts the connection.

### The response is `401`

The plugin's value must equal Kokoro's resolved token. Don't paste `Bearer` in
the plugin field. If both the Kokoro saved field and `KOKORO_WEBHOOK_TOKEN`
exist, the saved field wins. Restart after changing environment variables.

### The response is `404`

Compare the full endpoint, including the leading slash in `endpoint_path`.
The default route is `/webhook/message`, and it is served only while the
Webhook platform is enabled.

### The response is `400`

Keep `Content-Type: application/json`. Verify valid JSON, valid base64 for
audio, and at least one enabled text/image/audio value. Empty text plus no
media is rejected.

### The response is `500`

Check the Kokoro LLM provider first. For incoming audio, check STT. For voice
replies, check TTS and both voice/audio toggles. A plugin connection can be
healthy even when generation fails inside Kokoro.

### A character sees the wrong history

Check `character_id`, `conversation_strategy`, sender IDs, group IDs, and
`unified_msg_origin`. The plugin default `character_id: kokoro` overrides the
Kokoro Webhook default. A strategy change intentionally creates a different
conversation mapping; it does not merge existing histories.

### Images or audio do not appear in AstrBot

Enable the corresponding plugin toggle. For audio output, also enable Kokoro
Webhook **Send voice replies (TTS)** and configure TTS. The plugin drops media
that is disabled by configuration before yielding the AstrBot reply chain.

## Local verification and publication status

Run the plugin tests from its directory:

```bash
cd integrations/astrbot-kokoro
python -m pip install -r requirements-dev.txt
pytest -q
```

The package currently has local source and mocked HTTP coverage. A separate
plugin repository, AstrBot marketplace listing, screenshots/demo, and a real
channel smoke test require maintainer credentials and a running AstrBot
instance. Their owner, date, URLs, version/channel fields, and evidence are
tracked as `Pending` in
[`docs/release-reviews/astrbot-publication.md`](../release-reviews/astrbot-publication.md).

Do not turn those rows into `Pass` based on local pytest output. The release
review must include the external repository URL, marketplace URL, AstrBot
version, tested channel, screenshots/demo, and the real response result.

## Related references

- [AstrBot plugin package README](../../integrations/astrbot-kokoro/README.md)
- [Kokoro API specification](../API%20specification.md#authenticated-generic-webhook)
- [Kokoro troubleshooting](../troubleshooting.md)
- [Kokoro quick start](../quick-start.md)
- [AstrBot publication review](../release-reviews/astrbot-publication.md)
