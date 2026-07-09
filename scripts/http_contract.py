"""Typed HTTP/JSON boundary for repository proof and capture scripts."""

from __future__ import annotations

import json
import time
from enum import Enum
from typing import Callable
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from json_contract import JsonContractError, JsonObject, JsonObjectReader, decode_json_object


class HttpMethod(str, Enum):
    GET = "GET"
    POST = "POST"


class HttpFailureKind(str, Enum):
    AUTH = "auth"
    QUOTA = "quota"
    CONFIG = "config"
    TRANSIENT = "transient"


class HttpRequestError(RuntimeError):
    """A classified non-success response from an HTTP JSON producer."""

    def __init__(self, kind: HttpFailureKind, message: str) -> None:
        super().__init__(message)
        self.kind = kind

    @property
    def retryable(self) -> bool:
        return self.kind is HttpFailureKind.TRANSIENT


class HttpHealthError(RuntimeError):
    """A service did not satisfy its health contract before the deadline."""


def _failure_kind(status_code: int) -> HttpFailureKind:
    if status_code in {401, 403}:
        return HttpFailureKind.AUTH
    if status_code == 429:
        return HttpFailureKind.QUOTA
    if status_code >= 500:
        return HttpFailureKind.TRANSIENT
    return HttpFailureKind.CONFIG


def request_json_object(
    method: HttpMethod,
    url: str,
    payload: JsonObject | None = None,
    timeout_s: float = 600.0,
) -> JsonObjectReader:
    data = None
    headers = {"Accept": "application/json"}
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = Request(url, data=data, headers=headers, method=method.value)
    try:
        with urlopen(request, timeout=timeout_s) as response:
            body = response.read().decode("utf-8")
    except HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        kind = _failure_kind(exc.code)
        raise HttpRequestError(
            kind,
            f"{kind.value}: {method.value} {url} failed status={exc.code}: {body[:500]}",
        ) from exc
    if not body:
        raise JsonContractError(f"{method.value} {url} returned an empty JSON response")
    return decode_json_object(body, f"{method.value} {url} response")


def wait_for_json_health(
    label: str,
    url: str,
    is_healthy: Callable[[JsonObjectReader], bool],
    timeout_s: float = 120.0,
) -> None:
    deadline = time.time() + timeout_s
    last_error = "no response"
    health_url = f"{url.rstrip('/')}/health"
    while time.time() < deadline:
        try:
            payload = request_json_object(HttpMethod.GET, health_url, timeout_s=10.0)
            if is_healthy(payload):
                return
            last_error = f"{health_url} returned an unhealthy payload"
        except HttpRequestError as exc:
            if not exc.retryable:
                raise
            last_error = str(exc)
        except (URLError, TimeoutError) as exc:
            last_error = str(exc)
        time.sleep(2.0)
    raise HttpHealthError(f"{label} not healthy at {url}: {last_error}")
