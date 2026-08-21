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
  folder  Take the newest image in a directory. For industrial smart cameras
          (Hikrobot MV-SC and the like) that speak GigE Vision or a vendor SDK
          rather than HTTP, but whose own software already saves every shot to
          disk. The share is the interface; we do not reimplement their SDK.

Everything is standard library: a factory-floor deployment should not need a
package index to fetch a picture.
"""

from __future__ import annotations

import base64
import socket
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path

DEFAULT_TIMEOUT = 8.0
MAX_BYTES = 12 * 1024 * 1024

# A JPEG starts FFD8 and ends FFD9. Used to know when a raw stream has delivered
# a whole frame, since a socket that stays open gives no other end signal.
JPEG_START = b"\xff\xd8"
JPEG_END = b"\xff\xd9"
PNG_START = b"\x89PNG\r\n\x1a\n"
BMP_START = b"BM"


@dataclass(frozen=True)
class CameraConfig:
    enabled: bool = False
    transport: str = "http"          # http | raw | folder
    host: str = ""
    port: int = 80
    path: str = "/snapshot.jpg"      # http only
    username: str = ""
    password: str = ""
    trigger: str = ""                # raw only: bytes to send before reading
    folder: str = ""                 # folder only: where the camera saves stills
    timeout: float = DEFAULT_TIMEOUT

    @property
    def url(self) -> str:
        path = self.path if self.path.startswith("/") else f"/{self.path}"
        return f"http://{self.host}:{self.port}{path}"

    @property
    def endpoint(self) -> str:
        if self.transport == "folder":
            return f"file://{self.folder}"
        return self.url if self.transport == "http" else f"tcp://{self.host}:{self.port}"

    @property
    def configured(self) -> bool:
        """Whether the source has the one field it cannot work without."""
        return bool(self.folder) if self.transport == "folder" else bool(self.host)


class StaleImage(RuntimeError):
    """The newest image in the folder is too old to be the box in front of you.

    Separate from CameraError because it is not a fault: the camera is fine, it
    just has not taken a picture recently. The screen stays blank and waits,
    rather than showing a photo of whatever was scanned before lunch — which is
    the one failure this whole flow exists to prevent.
    """

    def __init__(self, message: str, *, age_seconds: float, source_time: str) -> None:
        super().__init__(message)
        self.age_seconds = age_seconds
        self.source_time = source_time


class CameraError(RuntimeError):
    """Reaching the camera failed. The message is meant to be shown to whoever
    is standing in front of it, so it names the address and what went wrong."""


def capture(config: CameraConfig) -> bytes:
    """Fetch one frame. Raises CameraError with something actionable."""
    if config.transport == "folder":
        if not config.folder:
            raise CameraError("尚未設定影像資料夾")
        return _capture_folder(config)
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


IMAGE_SUFFIXES = {".jpg", ".jpeg", ".png", ".bmp"}

# How long to let a file settle. The camera's own software is writing it, and a
# still that is half on disk decodes as a grey band — which reads as a bad photo
# rather than a race, so nobody thinks to look here.
SETTLE_TRIES = 15
SETTLE_WAIT = 0.2

# Older than this and the shot is not the box in front of you. Five minutes is
# long enough to cover walking the box over and getting the label straight, and
# short enough that the previous pallet's photo cannot stand in for this one.
STALE_AFTER = 300.0


def _mtime_iso(path: Path) -> str:
    return datetime.fromtimestamp(path.stat().st_mtime).isoformat(timespec="seconds")


def newest_image(config: CameraConfig) -> tuple[Path, float]:
    """The newest image in the folder, and how many seconds old it is.

    Age is returned rather than judged: a stale shot is not an error the way an
    unreachable camera is — the operator can see the picture and decide. But it
    has to reach the screen, because silently recognising an hour-old photo is
    how someone confirms an issue against a box that is no longer there.
    """
    folder = Path(config.folder)
    if not folder.is_dir():
        raise CameraError(f"找不到影像資料夾：{config.folder}")

    try:
        shots = [p for p in folder.iterdir()
                 if p.is_file() and p.suffix.lower() in IMAGE_SUFFIXES]
    except OSError as exc:
        raise CameraError(f"讀不到影像資料夾 {config.folder} — {exc}") from exc

    if not shots:
        raise CameraError(f"資料夾裡還沒有任何影像：{config.folder}")

    newest = max(shots, key=lambda p: p.stat().st_mtime)
    return newest, time.time() - newest.stat().st_mtime


def _capture_folder(config: CameraConfig) -> bytes:
    newest, age = newest_image(config)
    if age > STALE_AFTER:
        raise StaleImage(
            f"最新的影像是 {int(age // 60)} 分鐘前存的（{newest.name}），先拍一張再試。",
            age_seconds=age,
            source_time=_mtime_iso(newest),
        )

    # Wait for the size to stop moving before reading, so a shot taken this
    # second is not read mid-write.
    last = -1
    for _ in range(SETTLE_TRIES):
        size = newest.stat().st_size
        if size == last and size > 0:
            break
        last = size
        time.sleep(SETTLE_WAIT)

    try:
        data = newest.read_bytes()
    except OSError as exc:
        raise CameraError(f"讀不到 {newest.name} — {exc}") from exc

    return _validate(data, config)


def _validate(data: bytes, config: CameraConfig) -> bytes:
    if not data:
        raise CameraError(f"相機 {config.endpoint} 沒有回傳資料")
    if len(data) > MAX_BYTES:
        raise CameraError(f"相機回傳超過 {MAX_BYTES // 1024 // 1024}MB，可能不是單張影像")
    # A camera writing to disk may well save PNG or BMP; over the wire a still
    # is a JPEG in practice. Both get checked, so "not an image" stays a real
    # error rather than reaching recognition as bytes it cannot decode.
    magic = (JPEG_START, PNG_START, BMP_START) if config.transport == "folder" else (JPEG_START,)
    if not data.startswith(magic):
        # Naming what did arrive turns "it didn't work" into something fixable —
        # usually the path is an HTML error page or a stream rather than a still.
        head = data[:80].decode("utf-8", "replace").strip().replace("\n", " ")
        kinds = "JPEG／PNG／BMP" if config.transport == "folder" else "JPEG"
        raise CameraError(f"讀到的不是 {kinds} 影像。開頭是：{head[:60]}")
    return data
