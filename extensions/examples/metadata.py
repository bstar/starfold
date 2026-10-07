#!/usr/bin/env python3
"""Minimal independent Preview v1 provider. Only open user-selected regular files."""
import json
import os
import struct
import sys


def exact(size):
    data = bytearray()
    while len(data) < size:
        part = sys.stdin.buffer.read(size - len(data))
        if not part:
            raise EOFError("truncated frame")
        data.extend(part)
    return bytes(data)


def read():
    first = sys.stdin.buffer.read(1)
    if not first:
        return None
    control, binary = struct.unpack(">II", first + exact(7))
    if control > 1024 * 1024 or binary:
        raise ValueError("invalid request frame")
    return json.loads(exact(control))


def reply(request, message):
    frame = {key: request[key] for key in ("session", "generation", "sequence")}
    frame["message"] = message
    data = json.dumps(frame).encode()
    if len(data) > 1024 * 1024:
        raise ValueError("reply exceeds limit")
    sys.stdout.buffer.write(struct.pack(">II", len(data), 0) + data)
    sys.stdout.buffer.flush()


negotiated = False
session = None
path = None
while (request := read()) is not None:
    message = request["message"]
    kind = message["type"]
    if kind == "hello":
        if negotiated or message["version"] != 1:
            raise ValueError("incompatible protocol")
        negotiated = True
        session = request["session"]
        reply(request, {"type": "hello", "version": 1, "provider": "notes",
                        "revision": "1", "capabilities": ["documents", "input"]})
    elif not negotiated or request["session"] != session:
        raise ValueError("handshake required")
    elif kind in ("close", "cancel"):
        reply(request, {"type": "closed"})
        break
    else:
        if kind == "open":
            path = bytes(message["path"])
        if path is None:
            raise ValueError("open required")
        fields = [["Name", os.path.basename(path).decode("utf-8", errors="replace")]]
        reply(request, {"type": "content", "presentation": {
            "kind": "Notes extension", "fields": fields, "pages": [],
            "total_pages": None, "next_page": None, "notice": "Example provider",
            "raster": None, "surface": None, "media": None, "actions": [], "keys": []}})
