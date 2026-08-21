"""Camera transport errors must be actionable, and a non-image must not pass."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "core"))

from urdwms_core.camera import CameraConfig, CameraError, _validate, capture


class Config(unittest.TestCase):
    def test_url_normalises_a_missing_slash(self):
        c = CameraConfig(host="192.168.1.50", port=8080, path="snapshot.jpg")
        self.assertEqual(c.url, "http://192.168.1.50:8080/snapshot.jpg")

    def test_raw_endpoint_is_host_and_port(self):
        c = CameraConfig(transport="raw", host="192.168.1.50", port=9000)
        self.assertEqual(c.endpoint, "tcp://192.168.1.50:9000")


class Validation(unittest.TestCase):
    CONFIG = CameraConfig(host="192.168.1.50")

    def test_a_jpeg_passes(self):
        data = b"\xff\xd8" + b"x" * 100 + b"\xff\xd9"
        self.assertEqual(_validate(data, self.CONFIG), data)

    def test_an_html_error_page_is_rejected_with_its_content(self):
        # The usual failure: the path returns a login page or a 404 body, and a
        # generic "failed" would send someone hunting the network instead of the
        # path.
        with self.assertRaises(CameraError) as ctx:
            _validate(b"<html><body>401 Unauthorized</body></html>", self.CONFIG)
        self.assertIn("不是 JPEG", str(ctx.exception))
        self.assertIn("401", str(ctx.exception))

    def test_empty_response(self):
        with self.assertRaises(CameraError):
            _validate(b"", self.CONFIG)

    def test_oversized_response(self):
        with self.assertRaises(CameraError) as ctx:
            _validate(b"\xff\xd8" + b"x" * (13 * 1024 * 1024), self.CONFIG)
        self.assertIn("MB", str(ctx.exception))


class Capture(unittest.TestCase):
    def test_no_host_configured(self):
        with self.assertRaises(CameraError) as ctx:
            capture(CameraConfig())
        self.assertIn("尚未設定", str(ctx.exception))

    def test_unknown_transport(self):
        with self.assertRaises(CameraError):
            capture(CameraConfig(host="10.0.0.1", transport="rtsp"))

    def test_unreachable_host_names_the_address(self):
        # Port 1 on localhost refuses immediately, so this stays fast.
        with self.assertRaises(CameraError) as ctx:
            capture(CameraConfig(host="127.0.0.1", port=1, timeout=1.0))
        self.assertIn("127.0.0.1:1", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
