"""Explicit one-time download/verification. Runtime never calls this module."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import urllib.request


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def prepare(root, lock):
    root.mkdir(parents=True, exist_ok=True)
    for asset in lock["assets"]:
        path = root / asset["file"]
        if path.is_file() and digest(path) == asset["sha256"]:
            print(f"Verified {path.name}", flush=True)
            continue
        partial = path.with_suffix(path.suffix + ".part")
        try:
            print(f"Downloading {path.name}", flush=True)
            with urllib.request.urlopen(asset["url"], timeout=120) as source, partial.open("wb") as output:
                shutil.copyfileobj(source, output, 1024 * 1024)
            if digest(partial) != asset["sha256"]:
                raise RuntimeError(f"SHA-256 mismatch: {path.name}; refusing to install")
            partial.replace(path)
        finally:
            partial.unlink(missing_ok=True)
    print("Published FP32 weights verified. Runtime converts the model to FP16 in memory; no ONNX export or modified weights.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    prepare(args.root, json.loads(Path(__file__).with_name("assets.json").read_text()))
