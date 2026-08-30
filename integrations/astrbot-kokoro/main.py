"""AstrBot adapter for Kokoro Engine's authenticated generic webhook."""

from __future__ import annotations

from collections.abc import Iterable
from typing import Any
from urllib.parse import urlparse

import httpx
import mimetypes
from astrbot.api import AstrBotConfig, star
from astrbot.api.event import AstrMessageEvent, filter
from astrbot.api.message_components import Image, Plain, Record


# pattern: Imperative Shell — AstrBot events, media conversion, and HTTP I/O.

DEFAULT_ENDPOINT = "http://127.0.0.1:8787/webhook/message"
DEFAULT_TIMEOUT_SECONDS = 30.0
MAX_REPLY_MEDIA = 8


def _config_value(config: AstrBotConfig | dict[str, Any] | None, key: str, default: Any) -> Any:
    if config is None:
        return default
    getter = getattr(config, "get", None)
    if callable(getter):
        return getter(key, default)
    return default


def _enabled(config: AstrBotConfig | dict[str, Any] | None, key: str, default: bool = True) -> bool:
    value = _config_value(config, key, default)
    return value if isinstance(value, bool) else default


def _clean_text(value: Any) -> str:
    return value.strip() if isinstance(value, str) else ""


def _safe_endpoint(value: Any) -> str:
    endpoint = _clean_text(value) or DEFAULT_ENDPOINT
    parsed = urlparse(endpoint)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise ValueError("Kokoro endpoint must be an absolute http(s) URL")
    return endpoint


def _media_format(component: Any) -> str:
    # AstrBot Record.convert_to_base64() provides the normalized WAV payload;
    # the source filename/MIME describes the original upload, not the bytes.
    return "wav"


def _image_mime_type(component: Any) -> str:
    configured = _clean_text(getattr(component, "mime_type", ""))
    if configured.startswith("image/"):
        return configured.split(";", 1)[0].strip()
    for attribute in ("file", "url", "path"):
        source = _clean_text(getattr(component, attribute, ""))
        if source.startswith("data:image/"):
            return source.split(";", 1)[0].removeprefix("data:").strip()
        suffix = source.split("?", 1)[0].rsplit(".", 1)[-1].lower() if "." in source else ""
        extension_mimes = {
            "jpg": "image/jpeg",
            "jpeg": "image/jpeg",
            "png": "image/png",
            "webp": "image/webp",
            "gif": "image/gif",
            "bmp": "image/bmp",
        }
        if suffix in extension_mimes:
            return extension_mimes[suffix]
        guessed = mimetypes.guess_type(source)[0]
        if guessed and guessed.startswith("image/"):
            return guessed
    return "image/jpeg"


def _event_platform(event: AstrMessageEvent) -> str:
    getter = getattr(event, "get_platform_id", None)
    if callable(getter):
        value = _clean_text(getter())
        if value:
            return value
    getter = getattr(event, "get_platform_name", None)
    if callable(getter):
        value = _clean_text(getter())
        if value:
            return value
    return "astrbot"


def _event_identity(event: AstrMessageEvent, strategy: str = "character_session") -> tuple[str, str, str]:
    """Return conversation type, conversation ID, and sender ID."""

    sender_getter = getattr(event, "get_sender_id", None)
    sender_id = _clean_text(sender_getter()) if callable(sender_getter) else ""
    session_id = _clean_text(getattr(event, "session_id", ""))
    private_getter = getattr(event, "is_private_chat", None)
    is_private = bool(private_getter()) if callable(private_getter) else True
    if is_private:
        conversation_id = sender_id or session_id or "unknown"
        if strategy == "platform_session":
            conversation_id = _clean_text(getattr(event, "unified_msg_origin", "")) or conversation_id
        return "private", conversation_id, sender_id or session_id or "unknown"

    group_getter = getattr(event, "get_group_id", None)
    group_id = _clean_text(group_getter()) if callable(group_getter) else ""
    group_id = group_id or session_id or "unknown"
    if strategy == "platform_session":
        group_id = _clean_text(getattr(event, "unified_msg_origin", "")) or group_id
    return "group", group_id, sender_id or "unknown"


async def _component_payload(
    event: AstrMessageEvent,
    config: AstrBotConfig | dict[str, Any] | None,
) -> tuple[str, list[str], dict[str, str] | None]:
    text_parts: list[str] = []
    images: list[str] = []
    audio: dict[str, str] | None = None
    components = event.get_messages() if callable(getattr(event, "get_messages", None)) else ()
    if not isinstance(components, Iterable) or isinstance(components, (str, bytes)):
        components = ()

    for component in components:
        if isinstance(component, Plain):
            if _enabled(config, "enable_text"):
                text = _clean_text(getattr(component, "text", ""))
                if text:
                    text_parts.append(text)
        elif isinstance(component, Image) and _enabled(config, "enable_images"):
            if len(images) >= MAX_REPLY_MEDIA:
                continue
            encoded = _clean_text(await component.convert_to_base64())
            if encoded:
                mime_type = _image_mime_type(component)
                images.append(f"data:{mime_type};base64,{encoded}")
        elif isinstance(component, Record) and _enabled(config, "enable_audio") and audio is None:
            encoded = _clean_text(await component.convert_to_base64())
            if encoded:
                audio = {"data_base64": encoded, "audio_format": _media_format(component)}

    if not text_parts:
        getter = getattr(event, "get_message_str", None)
        fallback = _clean_text(getter()) if callable(getter) else _clean_text(getattr(event, "message_str", ""))
        if fallback and _enabled(config, "enable_text"):
            text_parts.append(fallback)
    return " ".join(text_parts), images, audio


def _webhook_payload(
    event: AstrMessageEvent,
    text: str,
    images: list[str],
    audio: dict[str, str] | None,
    character_id: str,
    conversation_strategy: str,
) -> dict[str, Any]:
    conversation_type, conversation_id, user_id = _event_identity(event, conversation_strategy)
    payload: dict[str, Any] = {
        "text": text,
        "conversation_type": conversation_type,
        "conversation_id": conversation_id,
        "user_id": user_id,
        "source": _event_platform(event),
    }
    if character_id:
        payload["character_id"] = character_id
    if images:
        payload["images"] = images
    if audio:
        payload["audio_base64"] = audio["data_base64"]
        payload["audio_format"] = audio["audio_format"]
    return payload


def _reply_chain(payload: Any, config: AstrBotConfig | dict[str, Any] | None) -> list[Any]:
    if not isinstance(payload, dict):
        raise ValueError("Kokoro response must be a JSON object")
    reply = payload.get("reply")
    if not isinstance(reply, str):
        raise ValueError("Kokoro response is missing a text reply")

    chain: list[Any] = []
    if reply:
        chain.append(Plain(reply))

    response_images = payload.get("images", [])
    if response_images is None:
        response_images = []
    if not isinstance(response_images, list):
        raise ValueError("Kokoro response images must be a list")
    if _enabled(config, "enable_images"):
        for item in response_images[:MAX_REPLY_MEDIA]:
            if not isinstance(item, dict) or not isinstance(item.get("data_base64"), str):
                raise ValueError("Kokoro response contains malformed image data")
            chain.append(Image.fromBase64(item["data_base64"]))

    response_audio = payload.get("audio")
    if response_audio is not None and _enabled(config, "enable_audio"):
        if not isinstance(response_audio, dict) or not isinstance(response_audio.get("data_base64"), str):
            raise ValueError("Kokoro response contains malformed audio data")
        chain.append(Record.fromBase64(response_audio["data_base64"]))

    if not chain:
        raise ValueError("Kokoro response has no supported content")
    return chain


@star.register("astrbot-kokoro", "Kokoro Engine Contributors", "Forward AstrBot messages to Kokoro Engine.", "0.1.0")
class Main(star.Star):
    """Forward every supported AstrBot message to Kokoro Engine."""

    def __init__(self, context: star.Context, config: AstrBotConfig | None = None) -> None:
        super().__init__(context, config)
        self.config = config if config is not None else {}

    @filter.event_message_type(filter.EventMessageType.ALL)
    async def on_message(self, event: AstrMessageEvent):
        """Forward supported text/media and yield an AstrBot message chain reply."""

        try:
            text, images, audio = await _component_payload(event, self.config)
            if not text and not images and audio is None:
                return
            endpoint = _safe_endpoint(_config_value(self.config, "endpoint", DEFAULT_ENDPOINT))
            character_id = _clean_text(_config_value(self.config, "character_id", ""))
            conversation_strategy = _clean_text(
                _config_value(self.config, "conversation_strategy", "character_session"),
            )
            if conversation_strategy not in {"character_session", "platform_session"}:
                conversation_strategy = "character_session"
            request_payload = _webhook_payload(
                event,
                text,
                images,
                audio,
                character_id,
                conversation_strategy,
            )
            headers = {"Content-Type": "application/json"}
            token = _clean_text(_config_value(self.config, "bearer_token", ""))
            if token:
                headers["Authorization"] = f"Bearer {token}"

            async with httpx.AsyncClient(timeout=DEFAULT_TIMEOUT_SECONDS) as client:
                response = await client.post(endpoint, headers=headers, json=request_payload)
            if response.status_code == 401:
                yield event.plain_result("Kokoro rejected the configured bearer token (HTTP 401).")
                return
            if response.status_code < 200 or response.status_code >= 300:
                yield event.plain_result(f"Kokoro returned HTTP {response.status_code}.")
                return
            try:
                response_payload = response.json()
                chain = _reply_chain(response_payload, self.config)
            except (TypeError, ValueError):
                yield event.plain_result("Kokoro returned an invalid response.")
                return
            yield event.chain_result(chain)
        except httpx.RequestError:
            yield event.plain_result("Could not connect to Kokoro.")
        except ValueError:
            yield event.plain_result("Kokoro endpoint configuration is invalid.")
