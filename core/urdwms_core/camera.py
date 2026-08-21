"""Pull a still frame from a network camera over TCP/IP.

The issuing flow does not care where an image comes from — it takes bytes. That
boundary is why a fixed camera can be added without touching recognition,
matching or the FIFO rules (M7 architecture: CaptureSource / CameraGateway).

Two transports, both plain TCP:

  http    GET a snapshot URL. What virtually every IP camera and most industrial
          cameras expose (`/snapshot.jpg`, `/cgi-bin/snapshot.cgi`, …). Optional
          basic auth.
  raw     Open a socket, optionally send a trigger string, read until the peer
          closes or the JPEG end marker appears. For cameras with a bare TCP
          command port and no HTTP layer.

Everything is standard library: a factory-floor deployment should not need a
package index to fetch a picture.
"""

from __future__ import annotations

import base64
import socket
import urllib.error
import urllib.request
from dataclasses import dataclass

DEFAULT_TIMEOUT = 8.0
MAX_BYTES = 12 * 1024 * 1024

# A JPEG starts FFD8 and ends FFD9. Used to know when a raw stream has delivered
# a whole frame, since a socket that stays open gives no other end signal.
JPEG_START = b"\xff\xd8"
JPEG_END = b"\xff\xd9"


@dataclass(frozen=True)
class CameraConfig:
    enabled: bool = False
    transport: str = "http"          # http | raw
    host: str = ""
    port: int = 80
    path: str = "/snapshot.jpg"      # http only
    username: str = ""
    password: str = ""
    trigger: str = ""                # raw only: bytes to send before reading
    timeout: float = DEFAULT_TIMEOUT

    @property
    def url(self) -> str:
        path = self.path if self.path.startswith("/") else f"/{self.path}"
        return f"http://{self.host}:{self.port}{path}"

    @property
    def endpoint(self) -> str:
        return self.url if self.transport == "http" else f"tcp://{self.host}:{self.port}"


class CameraError(RuntimeError):
    """Reaching the camera failed. The message is meant to be shown to whoever
    is standing in front of it, so it names the address and what went wrong."""


def capture(config: CameraConfig) -> bytes:
    """Fetch one frame. Raises CameraError with something actionable."""
    if not config.host:
        raise CameraError("尚未設定相機位址")
    if config.transport == "http":
        return _capture_http(config)
    if config.transport == "raw":
        return _capture_raw(config)
    raise CameraError(f"未知的連線方式：{config.transport}")


def _capture_http(config: CameraConfig) -> bytes:
    request = urllib.request.Request(config.url, method="GET")
    if config.username:
        token = base64.b64encode(f"{config.username}:{config.password}".encode()).decode()
        request.add_header("Authorization", f"Basic {token}")
    try:
        with urllib.request.urlopen(request, timeout=config.timeout) as response:
            data = response.read(MAX_BYTES + 1)
    except urllib.error.HTTPError as exc:
        raise CameraError(f"相機回應 HTTP {exc.code}（{config.url}）") from exc
    except urllib.error.URLError as exc:
        raise CameraError(f"連不到相機 {config.host}:{config.port} — {exc.reason}") from exc
    except TimeoutError as exc:
        raise CameraError(f"相機 {config.host}:{config.port} 逾時（{config.timeout:g} 秒）") from exc
    return _validate(data, config)


def _capture_raw(config: CameraConfig) -> bytes:
    try:
        with socket.create_connection((config.host, config.port), timeout=config.timeout) as sock:
            sock.settimeout(config.timeout)
            if config.trigger:
                sock.sendall(config.trigger.encode())
            chunks: list[bytes] = []
            total = 0
            while total <= MAX_BYTES:
                chunk = sock.recv(65536)
                if not chunk:
                    break
                chunks.append(chunk)
                total += len(chunk)
                # Stop at the frame boundary rather than waiting for a close the
                # camera may never perform.
                if JPEG_END in chunk:
                    break
    except socket.timeout as exc:
        raise CameraError(f"相機 {config.host}:{config.port} 逾時（{config.timeout:g} 秒）") from exc
    except OSError as exc:
        raise CameraError(f"連不到相機 {config.host}:{config.port} — {exc}") from exc
    return _validate(b"".join(chunks), config)


def _validate(data: bytes, config: CameraConfig) -> bytes:
    if not data:
        raise CameraError(f"相機 {config.endpoint} 沒有回傳資料")
    if len(data) > MAX_BYTES:
        raise CameraError(f"相機回傳超過 {MAX_BYTES // 1024 // 1024}MB，可能不是單張影像")
    if not data.startswith(JPEG_START):
        # Naming what did arrive turns "it didn't work" into something fixable —
        # usually the path is an HTML error page or a stream rather than a still.
        head = data[:80].decode("utf-8", "replace").strip().replace("\n", " ")
        raise CameraError(f"回傳的不是 JPEG 影像。開頭是：{head[:60]}")
    return data
