#!/usr/bin/env python3
"""Verify streamed, saved, embedded, and web images in the native transcript."""
import base64
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
from threading import Thread
import zlib

from desktop_check import ROOT, launch_thread, run_window, send_message


def fixture_png():
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    row = b"\x00" + b"\x0e\x99\x76" * 144
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">2I5B", 144, 32, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(row * 32))
        + chunk(b"IEND", b"")
    )


def main(executable):
    png = fixture_png()
    requests = []

    class ImageHandler(BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(png)))
            self.end_headers()
            self.wfile.write(png)

    with tempfile.TemporaryDirectory() as tmp, ThreadingHTTPServer(("127.0.0.1", 0), ImageHandler) as server:
        worker = Thread(target=server.serve_forever)
        worker.start()
        try:
            case = Path(tmp)
            checkout = case / "checkout"
            subprocess.run(["git", "init", "-q", checkout], check=True)
            (checkout / ".git/info/exclude").write_text("*.png\n")
            (checkout / "workspace chart.png").write_bytes(png)
            (case / "outside.png").write_bytes(png)
            (checkout / "outside.png").symlink_to(case / "outside.png")
            embedded = base64.b64encode(png).decode()
            absolute = (checkout / "workspace chart.png").as_uri()
            web_url = f"http://127.0.0.1:{server.server_port}/chart"
            link = "https://example.com/chart-details"
            first = "Here is the **chart**. ![Workspace chart](<workspace chart.png>) Look below."
            streaming = f"{first}\n\n![Streamed web chart]({web_url})"
            text = (
                f"{first}\n\n"
                f"- [![Web chart][web]]({link})\n\n"
                f"> ![Embedded chart](data:image/png;base64,{embedded})\n\n"
                f"![Absolute checkout path]({absolute})\n\n"
                "![Outside checkout](outside.png)\n\n"
                "![Missing chart](missing.png)\n\n"
                '`![Code stays text](<workspace chart.png>)`\n\n'
                f"> [web]: {web_url}"
            )
            env = dict(
                os.environ,
                HUGGLER_CODEX=str(ROOT / "tests/fixtures/desktop-images-peer.py"),
                HUGGLER_FIXTURE_IMAGE_RESPONSE=text,
                HUGGLER_FIXTURE_IMAGE_STREAM=streaming,
            )
            state = case / "state"
            run_window(executable, state, [
                *launch_thread(checkout, "Response images", current=True),
                *send_message("Show the charts"),
                {"see": "writing/" + streaming},
                {"see": "response-image/Workspace chart"},
                {"click": "load-response-image/Streamed web chart"},
                {"see": "response-image/Streamed web chart"},
                {"see": "Ready/Response images/Completed"},
                {"see": "response-image/Web chart"},
                {"click": "response-image/Web chart"},
                {"opened_url": link},
                {"see": "response-image/Workspace chart"},
                {"see": "response-image/Embedded chart"},
                {"see": "response-image/Absolute checkout path"},
                {"see": "response-image-status/Outside checkout/Image unavailable"},
                {"see": "response-image-status/Missing chart/Image unavailable"},
                {"absent": "response-image/Code stays text"},
                {"absent": "load-response-image/Web chart"},
            ], env)
            assert requests == ["/chart"], requests

            run_window(executable, state, [
                {"see": "response-image/Workspace chart"},
                {"see": "response-image/Embedded chart"},
                {"click": "load-response-image/Web chart"},
                {"see": "response-image/Web chart"},
                {"resize": [640, 540]},
                {"see": "response-image/Web chart"},
                {"min_height": ["response-image/Web chart", 16]},
                {"max_height": ["response-image/Web chart", 40]},
            ], env)
            assert requests == ["/chart", "/chart"], requests
        finally:
            server.shutdown()
            worker.join()


if __name__ == "__main__":
    main(Path(sys.argv[1]).resolve())
