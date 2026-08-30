"""Behavior tests for the AstrBot -> Kokoro webhook adapter."""

# pattern: Imperative Shell - test orchestration and mocked network boundary.

from __future__ import annotations

import importlib
from typing import Any

import httpx
import pytest

from .fakes import FakeContext, FakeEvent, Image, Plain, Record, install_astrbot_fakes


@pytest.fixture()
def plugin_module():
    install_astrbot_fakes()
    try:
        return importlib.import_module("main")
    except ModuleNotFoundError as error:
        pytest.fail(f"Kokoro AstrBot plugin is not implemented yet: {error}")


class FakeResponse:
    def __init__(self, status_code: int, payload: Any = None, *, json_error: Exception | None = None) -> None:
        self.status_code = status_code
        self._payload = payload
        self._json_error = json_error

    def json(self) -> Any:
        if self._json_error:
            raise self._json_error
        return self._payload


class FakeAsyncClient:
    def __init__(self, response: FakeResponse | None = None, error: Exception | None = None) -> None:
        self.response = response
        self.error = error
        self.calls: list[dict[str, Any]] = []

    async def __aenter__(self) -> "FakeAsyncClient":
        return self

    async def __aexit__(self, *_: Any) -> None:
        return None

    async def post(self, url: str, **kwargs: Any) -> FakeResponse:
        self.calls.append({"url": url, **kwargs})
        if self.error:
            raise self.error
        assert self.response is not None
        return self.response


def config() -> dict[str, Any]:
    return {
        "endpoint": "http://127.0.0.1:8787/webhook/message",
        "bearer_token": "test-secret",
        "character_id": "kokoro",
        "conversation_strategy": "character_session",
        "enable_text": True,
        "enable_images": True,
        "enable_audio": True,
    }


async def collect(handler, event: FakeEvent):
    return [result async for result in handler(event)]


@pytest.mark.asyncio
async def test_forwards_text_with_private_character_session(plugin_module, monkeypatch):
    client = FakeAsyncClient(FakeResponse(200, {"reply": "Hello from Kokoro"}))
    monkeypatch.setattr(plugin_module.httpx, "AsyncClient", lambda **_: client)
    plugin = plugin_module.Main(FakeContext(), config())

    results = await collect(plugin.on_message, FakeEvent([Plain("hello")], message_str="hello"))

    request = client.calls[0]
    assert request["url"] == config()["endpoint"]
    assert request["headers"] == {"Authorization": "Bearer test-secret", "Content-Type": "application/json"}
    assert request["json"] == {
        "text": "hello",
        "character_id": "kokoro",
        "conversation_type": "private",
        "conversation_id": "user-1",
        "user_id": "user-1",
        "source": "test",
    }
    assert results[0].chain[0].text == "Hello from Kokoro"


@pytest.mark.asyncio
async def test_reports_unauthorized_without_exposing_token(plugin_module, monkeypatch):
    client = FakeAsyncClient(FakeResponse(401, {"error": "unauthorized"}))
    monkeypatch.setattr(plugin_module.httpx, "AsyncClient", lambda **_: client)
    plugin = plugin_module.Main(FakeContext(), config())

    results = await collect(plugin.on_message, FakeEvent([Plain("hello")], message_str="hello"))

    assert "test-secret" not in results[0].chain[0].text
    assert "401" in results[0].chain[0].text


@pytest.mark.asyncio
async def test_reports_connection_failure(plugin_module, monkeypatch):
    request = httpx.Request("POST", config()["endpoint"])
    client = FakeAsyncClient(error=httpx.ConnectError("offline", request=request))
    monkeypatch.setattr(plugin_module.httpx, "AsyncClient", lambda **_: client)
    plugin = plugin_module.Main(FakeContext(), config())

    results = await collect(plugin.on_message, FakeEvent([Plain("hello")], message_str="hello"))

    assert "connect" in results[0].chain[0].text.lower()


@pytest.mark.asyncio
async def test_reports_malformed_reply(plugin_module, monkeypatch):
    client = FakeAsyncClient(FakeResponse(200, json_error=ValueError("not json")))
    monkeypatch.setattr(plugin_module.httpx, "AsyncClient", lambda **_: client)
    plugin = plugin_module.Main(FakeContext(), config())

    results = await collect(plugin.on_message, FakeEvent([Plain("hello")], message_str="hello"))

    assert "response" in results[0].chain[0].text.lower()


@pytest.mark.asyncio
async def test_maps_group_and_private_media_to_supported_webhook_fields(plugin_module, monkeypatch):
    client = FakeAsyncClient(FakeResponse(200, {"reply": "media received"}))
    monkeypatch.setattr(plugin_module.httpx, "AsyncClient", lambda **_: client)
    plugin = plugin_module.Main(FakeContext(), config())
    event = FakeEvent(
        [
            Plain("look"),
            Image(encoded="aW1hZ2U=", mime_type="image/png"),
            Record(encoded="YXVkaW8=", mime_type="audio/ogg"),
        ],
        private=False,
        group_id="group-9",
        message_str="look",
    )

    await collect(plugin.on_message, event)

    payload = client.calls[0]["json"]
    assert payload["conversation_type"] == "group"
    assert payload["conversation_id"] == "group-9"
    assert payload["images"] == ["data:image/png;base64,aW1hZ2U="]
    assert payload["audio_base64"] == "YXVkaW8="
    assert payload["audio_format"] == "wav"


@pytest.mark.asyncio
async def test_platform_session_uses_platform_origin_instead_of_character_session(plugin_module, monkeypatch):
    client = FakeAsyncClient(FakeResponse(200, {"reply": "ok"}))
    monkeypatch.setattr(plugin_module.httpx, "AsyncClient", lambda **_: client)
    plugin = plugin_module.Main(FakeContext(), config() | {"conversation_strategy": "platform_session"})
    event = FakeEvent([Plain("hello")], message_str="hello")
    event.unified_msg_origin = "test:private:user-1"

    await collect(plugin.on_message, event)

    payload = client.calls[0]["json"]
    assert payload["conversation_type"] == "private"
    assert payload["conversation_id"] == "test:private:user-1"
    assert payload["conversation_id"] != "user-1"


@pytest.mark.asyncio
async def test_image_without_mime_type_uses_file_extension(plugin_module, monkeypatch):
    client = FakeAsyncClient(FakeResponse(200, {"reply": "image received"}))
    monkeypatch.setattr(plugin_module.httpx, "AsyncClient", lambda **_: client)
    plugin = plugin_module.Main(FakeContext(), config())
    image = Image(file="/tmp/reference.webp", encoded="cGlj")
    del image.mime_type

    await collect(plugin.on_message, FakeEvent([image], message_str=""))

    assert client.calls[0]["json"]["images"] == ["data:image/webp;base64,cGlj"]


@pytest.mark.asyncio
async def test_returns_text_image_and_audio_components(plugin_module, monkeypatch):
    client = FakeAsyncClient(
        FakeResponse(
            200,
            {
                "reply": "Here are the files",
                "images": [{"data_base64": "cGlj", "mime_type": "image/webp", "file_name": "pic.webp"}],
                "audio": {"data_base64": "dm9pY2U=", "mime_type": "audio/ogg", "file_name": "voice.ogg"},
            },
        ),
    )
    monkeypatch.setattr(plugin_module.httpx, "AsyncClient", lambda **_: client)
    plugin = plugin_module.Main(FakeContext(), config())

    results = await collect(plugin.on_message, FakeEvent([Plain("files")], message_str="files"))

    assert [component.type for component in results[0].chain] == ["Plain", "Image", "Record"]
    assert results[0].chain[0].text == "Here are the files"
    assert results[0].chain[1].encoded == "cGlj"
    assert results[0].chain[2].encoded == "dm9pY2U="
