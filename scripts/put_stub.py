#!/usr/bin/env python3
"""Local HTTP PUT/HEAD/GET stub for ChunkForge push demos (no internet).

Mirrors the default CAS layout under --root: request path
``/chunks/<2hex>/<62hex>.cnk`` → ``<root>/chunks/<2hex>/<62hex>.cnk``.

Supports PUT (and POST) to write bodies, HEAD for presence, GET for verify/cat.
Optional ``--fail-transient N``: first N PUT/POST attempts return HTTP 503
(body still consumed), then behave normally — for Phase 8 HTTP-retry demos.
Not a production object store — Phase 4+ smoke only.
"""

from __future__ import annotations

import argparse
import http.server
import os
import sys
import threading
from pathlib import Path


class PutStubHandler(http.server.BaseHTTPRequestHandler):
    root: Path  # set on the class before serving
    fail_lock = threading.Lock()
    fail_remaining: int = 0

    def log_message(self, fmt: str, *args) -> None:  # quieter than default
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))

    def _rel_path(self) -> Path:
        # Strip query; map URL path onto --root (no path traversal)
        path = self.path.split("?", 1)[0]
        rel = path.lstrip("/")
        dest = (self.root / rel).resolve()
        try:
            dest.relative_to(self.root.resolve())
        except ValueError as exc:
            raise PermissionError(f"path escapes root: {path}") from exc
        return dest

    def _consume_fail_transient(self) -> bool:
        """Return True if this PUT should be answered with 503."""
        with self.fail_lock:
            if self.fail_remaining > 0:
                PutStubHandler.fail_remaining -= 1
                return True
        return False

    def do_HEAD(self) -> None:  # noqa: N802
        try:
            dest = self._rel_path()
        except PermissionError:
            self.send_error(403)
            return
        if dest.is_file():
            size = dest.stat().st_size
            self.send_response(200)
            self.send_header("Content-Length", str(size))
            self.send_header("Content-Type", "application/octet-stream")
            self.end_headers()
        else:
            self.send_response(404)
            self.end_headers()

    def do_GET(self) -> None:  # noqa: N802
        try:
            dest = self._rel_path()
        except PermissionError:
            self.send_error(403)
            return
        if not dest.is_file():
            self.send_response(404)
            self.end_headers()
            return
        data = dest.read_bytes()
        self.send_response(200)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Content-Type", "application/octet-stream")
        self.end_headers()
        self.wfile.write(data)

    def do_PUT(self) -> None:  # noqa: N802
        self._write_body()

    def do_POST(self) -> None:  # noqa: N802
        self._write_body()

    def _write_body(self) -> None:
        try:
            dest = self._rel_path()
        except PermissionError:
            self.send_error(403)
            return
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length) if length > 0 else b""
        # Consume body before deciding 503 so ureq does not hang.
        if self._consume_fail_transient():
            self.send_response(503)
            self.send_header("Content-Length", "0")
            self.end_headers()
            sys.stderr.write("put_stub: injected 503 (fail-transient)\n")
            return
        dest.parent.mkdir(parents=True, exist_ok=True)
        # Atomic-ish: write temp then rename
        tmp = dest.with_suffix(dest.suffix + ".tmp")
        tmp.write_bytes(body)
        os.replace(tmp, dest)
        self.send_response(200)
        self.send_header("Content-Length", "0")
        self.end_headers()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--root",
        type=Path,
        required=True,
        help="Directory that receives chunks/<2hex>/<62hex>.cnk",
    )
    ap.add_argument(
        "--port",
        type=int,
        default=8766,
        help="Listen port (default 8766)",
    )
    ap.add_argument(
        "--bind",
        default="127.0.0.1",
        help="Bind address (default 127.0.0.1)",
    )
    ap.add_argument(
        "--fail-transient",
        type=int,
        default=0,
        metavar="N",
        help="First N PUT/POST attempts return HTTP 503 (default 0)",
    )
    args = ap.parse_args()

    if args.fail_transient < 0:
        print("error: --fail-transient must be >= 0", file=sys.stderr)
        return 2

    root = args.root.resolve()
    root.mkdir(parents=True, exist_ok=True)
    PutStubHandler.root = root
    PutStubHandler.fail_remaining = args.fail_transient

    server = http.server.ThreadingHTTPServer((args.bind, args.port), PutStubHandler)
    print(
        f"put_stub: listening on http://{args.bind}:{args.port} root={root}"
        f" fail_transient={args.fail_transient}",
        flush=True,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nput_stub: stopped", flush=True)
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
