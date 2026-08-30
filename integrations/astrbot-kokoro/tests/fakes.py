"""Small AstrBot API fakes used by the plugin's HTTP contract tests."""

# pattern: Imperative Shell - test-only AstrBot API boundary fakes.

from __future__ import annotations

import sys
import types
from dataclasses import dataclass
from typing import Any


class _Star:
    def __init__(self, context: Any, config: Any = None) -> None:
        self.context = context
        self.config = config


class _Plain:
    def __init__(self, text: str, **_: Any) -> None:
        self.text = text
        self.type = "Plain"


class _Image:
    def __init__(self, file: str = "", *, encoded: str = "", mime_type: str = "image/jpeg", **_: Any) -> None:
        self.file = file
        self.encoded = encoded
        self.mime_type = mime_type
        self.type = "Image"

    async def convert_to_base64(self) -> str:
        return self.encoded

    @staticmethod
    def fromBase64(value: str, **_: Any) -> "_Image":
        return _Image(encoded=value)


class _Record:
    def __init__(self, file: str = "", *, encoded: str = "", mime_type: str = "audio/ogg", **_: Any) -> None:
        self.file = file
        self.encoded = encoded
        self.mime_type = mime_type
        self.type = "Record"

    async def convert_to_base64(self) -> str:
        return self.encoded

    @staticmethod
    def fromBase64(value: str, **_: Any) -> "_Record":
        return _Record(encoded=value)


@dataclass
class FakeResult:
    chain: list[Any]


class FakeEvent:
    def __init__(
        self,
        components: list[Any],
        *,
        private: bool = True,
        platform: str = "test",
        sender_id: str = "user-1",
        group_id: str = "",
        message_str: str = "",
    ) -> None:
        self._components = components
        self._private = private
        self._platform = platform
        self._sender_id = sender_id
        self._group_id = group_id
        self.message_str = message_str
        self.session_id = group_id or sender_id

    def get_messages(self) -> list[Any]:
        return self._components

    def get_message_str(self) -> str:
        return self.message_str

    def is_private_chat(self) -> bool:
        return self._private

    def get_sender_id(self) -> str:
        return self._sender_id

    def get_group_id(self) -> str:
        return self._group_id

    def get_platform_id(self) -> str:
        return self._platform

    def plain_result(self, text: str) -> FakeResult:
        return FakeResult([_Plain(text)])

    def chain_result(self, chain: list[Any]) -> FakeResult:
        return FakeResult(chain)


class FakeContext:
    def get_config(self, **_: Any) -> dict[str, Any]:
        return {}


class _Filter:
    class EventMessageType:
        ALL = "all"

    @staticmethod
    def event_message_type(*_: Any, **__: Any):
        def decorator(func):
            return func

        return decorator

    @staticmethod
    def platform_adapter_type(*_: Any, **__: Any):
        def decorator(func):
            return func

        return decorator


def install_astrbot_fakes() -> None:
    """Install only the documented imports used by ``main.py``."""

    astrbot = types.ModuleType("astrbot")
    api = types.ModuleType("astrbot.api")
    api_star = types.ModuleType("astrbot.api.star")
    api_event = types.ModuleType("astrbot.api.event")
    api_components = types.ModuleType("astrbot.api.message_components")
    core = types.ModuleType("astrbot.core")
    core_config = types.ModuleType("astrbot.core.config")

    api_star.Star = _Star
    api_star.register = lambda *_args, **_kwargs: (lambda cls: cls)
    api_event.AstrMessageEvent = FakeEvent
    api_event.filter = _Filter
    api_components.Plain = _Plain
    api_components.Image = _Image
    api_components.Record = _Record
    core_config.AstrBotConfig = dict
    api.star = api_star
    api.AstrBotConfig = dict
    astrbot.api = api
    astrbot.core = core

    sys.modules.update(
        {
            "astrbot": astrbot,
            "astrbot.api": api,
            "astrbot.api.star": api_star,
            "astrbot.api.event": api_event,
            "astrbot.api.message_components": api_components,
            "astrbot.core": core,
            "astrbot.core.config": core_config,
        },
    )


Plain = _Plain
Image = _Image
Record = _Record
