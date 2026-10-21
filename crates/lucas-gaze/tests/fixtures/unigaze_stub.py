"""Numeric-only test peer for the actual child-process pipe boundary; no ML deps."""
import json
import struct
import sys

mode = sys.argv[1]
print(json.dumps({"ready": True, "provider": "CUDA cuda:0", "gpu": "TEST PEER, not a GPU"}), flush=True)
while True:
    header = sys.stdin.buffer.read(20)
    if not header:
        break
    magic, w, h, at = struct.unpack("<4sIIQ", header)
    if magic != b"LUG1":
        raise SystemExit(2)
    pixels = sys.stdin.buffer.read(w*h*3)
    if len(pixels) != w*h*3:
        raise SystemExit(3)
    if mode == "crash":
        raise SystemExit(4)
    if mode == "wrong_timestamp":
        at += 1
    values = None if mode == "unknown" else [0.1, 0.2, 0.2, 0.2, 0, 0, 0, 10, 20, 650]
    print(json.dumps({"timestamp_ns": at, "features": values,
                      "quality": 0.0 if values is None else 0.9,
                      "face_found": values is not None, "luminance": 0.5}), flush=True)
