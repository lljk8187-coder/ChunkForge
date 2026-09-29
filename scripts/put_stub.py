#!/usr/bin/env python3
"""Local HTTP PUT/HEAD/GET stub for ChunkForge push demos (no internet).

Mirrors the default CAS layout under --root: request path
``/chunks/<2hex>/<62hex>.cnk`` → ``<root>/chunks/<2hex>/<62hex>.cnk``.

Supports PUT (and POST) to write bodies, HEAD for presence, GET for verify/cat.
Not a production object store — Phase 4 M4 smoke only.
"""

from __future__ import annotations

import argparse
import http.server
import os
import sys
from pathlib import Path


class PutStubHandler(http.server.BaseHTTPRequestHandler):
    root: Path  # set on the class before serving

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
    args = ap.parse_args()

    root = args.root.resolve()
    root.mkdir(parents=True, exist_ok=True)
    PutStubHandler.root = root

    server = http.server.ThreadingHTTPServer((args.bind, args.port), PutStubHandler)
    print(
        f"put_stub: listening on http://{args.bind}:{args.port} root={root}",
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
