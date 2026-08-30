# Kokoro Engine for AstrBot

This plugin forwards AstrBot messages to Kokoro Engine's authenticated generic
webhook. Kokoro remains the character, conversation, LLM, TTS, and media
response engine; AstrBot remains the channel adapter.

The plugin targets AstrBot `>=4.5.0` and uses the public `Star`,
`AstrBotConfig`, `AstrMessageEvent`, and message-component APIs. It does not
open a second server or expose Kokoro credentials to AstrBot channels.

## Before you install

You need:

- A running Kokoro Engine instance with an LLM provider configured.
- Kokoro's **Settings > Bots > Webhook** runtime enabled and started.
- An AstrBot `>=4.5.0` instance with this plugin installed.
- A shared Bearer token if the Kokoro webhook token is enabled. Use one even on
  a trusted LAN; an unauthenticated webhook accepts requests from any client
  that can reach its bind address.

The plugin source is currently in this repository at
[`integrations/astrbot-kokoro`](https://github.com/chyinan/Kokoro-Engine/tree/main/integrations/astrbot-kokoro).
The separate plugin repository and marketplace listing are tracked as pending
in [`docs/release-reviews/astrbot-publication.md`](../../docs/release-reviews/astrbot-publication.md).

## Install the plugin

For a development install, copy the directory containing `main.py`,
`metadata.yaml`, `_conf_schema.json`, and `LICENSE` into the plugin directory
used by your AstrBot instance. Restart AstrBot and enable **Kokoro Engine** in
the plugin manager. When the separate marketplace listing is available, use
AstrBot's normal marketplace install flow instead.

Do not copy `tests/` into a production plugin directory unless you are running
the local test suite.

## Configure Kokoro

In Kokoro, open **Settings > Bots > Webhook**:

1. Turn on the Webhook platform.
2. Keep `Bind host` as `127.0.0.1` when AstrBot runs on the same host. Keep the
   default port `8787` and endpoint path `/webhook/message`, or note the values
   if you changed them.
3. Set a Bearer token in Kokoro, or set `KOKORO_WEBHOOK_TOKEN` before starting
   Kokoro. Kokoro uses the saved token first and then the environment variable.
4. Select the default character for requests that do not name a character.
5. Enable **Send voice replies (TTS)** only when Kokoro has a working TTS
   provider. Save the settings and start the Webhook runtime.

The webhook is a `POST` endpoint. Its response is JSON with a required string
`reply`, optional `images` (`data_base64` objects), and optional `audio`
(`data_base64` object). See the complete [authenticated generic webhook
contract](../../docs/API%20specification.md#authenticated-generic-webhook).

## Configure the AstrBot plugin

Open the plugin configuration and set these fields:

| Field | Default | What it controls |
| --- | --- | --- |
| `endpoint` | `http://127.0.0.1:8787/webhook/message` | Kokoro's full HTTP URL, including the path. |
| `bearer_token` | empty | Must match Kokoro's token. Leave empty only when Kokoro auth is intentionally disabled. |
| `character_id` | `kokoro` | Per-request character override. Clear it to use Kokoro's own default/active selection. |
| `conversation_strategy` | `character_session` | Stable mapping for private and group conversations. |
| `enable_text` | `true` | Forward plain text and deliver text replies. |
| `enable_images` | `true` | Forward images and deliver image replies. |
| `enable_audio` | `true` | Forward audio for Kokoro STT and deliver Kokoro audio replies. |

The plugin never logs the token. Keep it in AstrBot's secret configuration and
do not paste it into issue reports or screenshots.

### Character selection precedence

Kokoro resolves the character for each request in this order:

1. Non-empty `character_id` sent by AstrBot.
2. Kokoro's Webhook `character_id` setting.
3. Kokoro's active character.
4. `default` if no character is available.

The plugin's default `character_id: kokoro` therefore wins over Kokoro's Webhook
default. To switch characters, set the plugin field to a catalog ID such as
`kokoro`, `pico`, or `seren`, then send a new message. To let Kokoro decide,
clear the plugin field. A character override is not a new account or a new
remote persona: Kokoro scopes the stored conversation to that character.

### Private and group conversations

With `character_session` (the default):

- A private message uses the AstrBot sender ID as `user_id` and
  `private:<sender-id>` as its conversation identity.
- A group message uses the AstrBot group ID as `conversation_id` and
  `group:<group-id>` as its conversation identity. All members of that group
  share the same character-owned history.

Kokoro adds the resolved character ID before persisting the conversation. A
private conversation for `kokoro` cannot be mixed with the same sender's
conversation for `pico`.

Choose `platform_session` when one AstrBot platform must keep its own unified
session identity. The plugin uses AstrBot's `unified_msg_origin` when it is
available and otherwise falls back to the sender or group identity. Keep one
strategy stable after deployment; changing it starts a different conversation
mapping.

### Text, image, and audio toggles

The toggles apply on both sides of the bridge:

- With text disabled, `Plain` components and text replies are ignored.
- With images disabled, AstrBot images are not sent and Kokoro image replies
  are not returned.
- With audio disabled, AstrBot `Record` components are not sent and Kokoro
  audio replies are not returned.

An AstrBot image is sent as a `data:image/...;base64,...` value. Audio is sent
as `audio_base64` plus a format inferred from its MIME type (`ogg`, `mp3`,
`wav`, `webm`, or `m4a`). Kokoro transcribes incoming audio before generating
the reply, so audio input requires a working STT configuration. Voice replies
are generated by Kokoro's **Send voice replies (TTS)** setting; the plugin's
audio toggle must also be enabled to deliver that `audio` object back to
AstrBot.

## Loopback and container endpoints

Use the endpoint that is reachable from the AstrBot process, not the endpoint
that is convenient from your browser.

| Deployment | AstrBot plugin endpoint | Kokoro bind guidance |
| --- | --- | --- |
| Both processes on the same host | `http://127.0.0.1:8787/webhook/message` | Keep `127.0.0.1`; this is the safest default. |
| AstrBot in Docker, Kokoro on the host | `http://host.docker.internal:8787/webhook/message` on Docker Desktop | Ensure the host service is reachable from the container. On Linux, add a host-gateway mapping or use a host-interface address. |
| Both services in one Docker network | `http://kokoro:8787/webhook/message` (replace `kokoro` with the service name) | Bind Kokoro to `0.0.0.0` inside its container and publish only the private Docker network. |
| Kokoro in Docker, AstrBot on the host | `http://127.0.0.1:8787/webhook/message` when the port is published to loopback | Publish with `127.0.0.1:8787:8787`; do not publish the token-protected service to the public interface unless a firewall and reverse proxy are configured. |

`127.0.0.1` inside a container means that container itself. A connection
refused error usually means the endpoint points at the wrong network namespace,
the Webhook runtime is stopped, or the port is not published. If you bind to
`0.0.0.0`, restrict the port at the host firewall and keep the Bearer token
enabled.

## Curl smoke tests

Set the token in the shell that runs `curl`; never replace it with a token in a
committed document. These commands exercise the same fields used by the
plugin. The success tests require a configured LLM, and the media tests also
require the corresponding Kokoro STT/TTS capability.

### Text and character selection

```bash
export KOKORO_WEBHOOK_TOKEN='replace-with-your-token'
export KOKORO_WEBHOOK_URL='http://127.0.0.1:8787/webhook/message'

curl --fail-with-body --silent --show-error \
  -X POST "$KOKORO_WEBHOOK_URL" \
  -H "Authorization: Bearer $KOKORO_WEBHOOK_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"text":"Hello from AstrBot","character_id":"kokoro","conversation_type":"private","user_id":"smoke-user-1","source":"astrbot-smoke"}'
```

The JSON response should contain a non-empty `reply`. Send the same request
again with the same `user_id` to exercise private conversation reuse. Change
`character_id` to `pico` and verify that the conversation is character-scoped.

### Group image request

This uses a 1x1 PNG data URL, so it does not require Kokoro to fetch an
external URL:

```bash
curl --fail-with-body --silent --show-error \
  -X POST "$KOKORO_WEBHOOK_URL" \
  -H "Authorization: Bearer $KOKORO_WEBHOOK_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"text":"Describe this image","images":["data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="],"conversation_type":"group","conversation_id":"smoke-group-1","user_id":"smoke-user-1","source":"astrbot-smoke"}'
```

### Audio request

Provide a real short audio file and set the matching format. The endpoint
accepts raw base64 or a `data:...;base64,...` value:

```bash
audio_base64="$(base64 -w 0 ./sample.ogg)"
curl --fail-with-body --silent --show-error \
  -X POST "$KOKORO_WEBHOOK_URL" \
  -H "Authorization: Bearer $KOKORO_WEBHOOK_TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"text\":\"\",\"audio_base64\":\"$audio_base64\",\"audio_format\":\"ogg\",\"conversation_type\":\"private\",\"user_id\":\"smoke-user-1\",\"source\":\"astrbot-smoke\"}"
```

### Authentication failure

This should return HTTP `401` and a JSON object containing `error` without
revealing the expected token:

```bash
curl --silent --show-error -o /tmp/kokoro-auth-error.json -w '%{http_code}\n' \
  -X POST "$KOKORO_WEBHOOK_URL" \
  -H 'Authorization: Bearer deliberately-wrong-token' \
  -H 'Content-Type: application/json' \
  -d '{"text":"auth smoke test","user_id":"smoke-user-1","conversation_type":"private"}'
```

Expected status: `401`.

## Troubleshooting

### `401 Unauthorized`

Compare AstrBot's `bearer_token` with Kokoro's saved Bearer token. If Kokoro
uses `KOKORO_WEBHOOK_TOKEN`, restart Kokoro after changing the environment.
Kokoro checks its saved token before the environment variable. Remove leading
or trailing whitespace and do not include the word `Bearer` in the configured
token value; the plugin adds that scheme to the HTTP header.

### `404 Not found`

The URL path must exactly match Kokoro's **Endpoint path**, including the
leading slash. The default is `/webhook/message`. Confirm the Webhook runtime
is enabled; a stopped runtime cannot serve the route.

### Connection refused or timeout

Check the Webhook runtime status, bind host, port, and container routing. From
the AstrBot environment, run `curl` against the configured URL. In a container,
replace `127.0.0.1` with `host.docker.internal` or the Kokoro service name as
described above. If the host is `0.0.0.0`, check the firewall rather than
making the service public by default.

### `400 Bad Request`

Kokoro returns `400` for invalid JSON, invalid base64, or a request with no
text or accepted media. Keep `Content-Type: application/json`, use the exact
field names above, and check that at least one enabled component is present.

### `500 Internal Server Error`

This indicates a Kokoro runtime failure, such as an unavailable LLM or STT
provider. An audio request needs STT; enabling voice replies also needs a
working TTS provider. Check Kokoro's provider status and retry the text smoke
test before debugging AstrBot.

### No reply or missing media

Check the three plugin toggles. A disabled media type is intentionally dropped
before the HTTP request or before the AstrBot reply chain is built. For voice
output, enable Kokoro's Webhook **Send voice replies (TTS)** and AstrBot's
`enable_audio` together.

## Related documentation

- [Kokoro integration guide](../../docs/integrations/astrbot.md)
- [Kokoro webhook API](../../docs/API%20specification.md#authenticated-generic-webhook)
- [Kokoro quick start](../../docs/quick-start.md)
- [General troubleshooting](../../docs/troubleshooting.md)
- [AstrBot release evidence](../../docs/release-reviews/astrbot-publication.md)
