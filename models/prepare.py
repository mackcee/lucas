#!/usr/bin/env python3
"""Development-only download/conversion. Never imported by the application.

Converts once and records the produced bytes in SHA256SUMS. Conversion is NOT
bit-reproducible: the lock pins shipped artifacts, not a reproducible-build claim.
Every invocation is a maintainer operation; review models and lock together.
"""
import argparse
import hashlib
import importlib.metadata
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import urllib.request
import zipfile

URL = "https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task"
ARCHIVE_SHA256 = "64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff"
SOURCES = {
    "face_detector": "b4578f35940bf5a1a655214a1cce5cab13eba73c1297cd78e1a04c2380b0152f",
    "face_landmarks_detector": "c7d54204ce0448474c7f3fa9af494787c0965cbdd6f20fc72867e43046bd43d5",
}
HERE = Path(__file__).resolve().parent


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=HERE)
    parser.add_argument("--compare", type=Path, help="check numerical equivalence against models in this directory before installing")
    args = parser.parse_args()
    for line in (HERE / "requirements.txt").read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        name, version = line.split("==")
        if importlib.metadata.version(name) != version:
            raise RuntimeError(f"Use pinned models/requirements.txt ({name}=={version})")
    if sys.version_info[:2] != (3, 12):
        raise RuntimeError("Use Python 3.12 for the pinned conversion environment")
    # Bound the archive and extract only two named files, never arbitrary ZIP paths.
    with urllib.request.urlopen(URL, timeout=60) as response:
        archive = response.read(16 * 1024 * 1024 + 1)
    if digest(archive) != ARCHIVE_SHA256:
        raise RuntimeError("Upstream task archive SHA-256 mismatch")
    env = dict(os.environ, PYTHONHASHSEED="0", TF_NUM_INTRAOP_THREADS="1", TF_NUM_INTEROP_THREADS="1", OMP_NUM_THREADS="1")
    with tempfile.TemporaryDirectory(prefix="lucas-models-") as tmp:
        tmp = Path(tmp)
        sums = []
        with zipfile.ZipFile(io.BytesIO(archive)) as bundle:
            for name, sha in SOURCES.items():
                tflite = bundle.read(name + ".tflite")
                if digest(tflite) != sha:
                    raise RuntimeError(f"Source checksum mismatch: {name}")
                source = tmp / (name + ".tflite")
                target = tmp / (name + ".onnx")
                source.write_bytes(tflite)
                subprocess.run([sys.executable, "-m", "tf2onnx.convert", "--tflite", str(source), "--opset", "17", "--output", str(target)], env=env, check=True)
                # Conversion changes serialization and precision, not model topology.
                import onnx
                model = onnx.load(str(target))
                # tf2onnx embeds the absolute temporary TFLite path here. Remove
                # this machine-specific annotation before hashing/distributing.
                model.graph.doc_string = ""
                onnx.checker.check_model(model)
                target.write_bytes(model.SerializeToString(deterministic=True))
                sums.append(f"{digest(target.read_bytes())}  {target.name}\n")
        lock = "".join(sums)
        if args.compare:
            compare_models(tmp, args.compare)
        args.output.mkdir(parents=True, exist_ok=True)
        for name in SOURCES:
            target = args.output / (name + ".onnx")
            staged = target.with_suffix(".onnx.tmp")
            staged.write_bytes((tmp / target.name).read_bytes())
            staged.replace(target)
        (args.output / "SHA256SUMS").write_text(lock)
        print(lock, end="")
        print("Review models and SHA256SUMS together; rebuild LUCAS after replacing the compiled lock.")


def compare_models(fresh, reference):
    """Check both networks on RGB portrait-derived inputs, not protobuf bytes."""
    import numpy as np
    import onnxruntime as ort
    from PIL import Image, ImageOps

    ort.disable_telemetry_events()
    options = ort.SessionOptions()
    options.intra_op_num_threads = 1
    options.inter_op_num_threads = 1
    portrait = HERE.parent / "crates/lucas-gaze/tests/fixtures/portrait.jpg"
    with Image.open(portrait) as image:
        image = image.convert("RGB")
        for name, size in (("face_detector", 128), ("face_landmarks_detector", 256)):
            crop = ImageOps.pad(image, (size, size)) if size == 128 else ImageOps.fit(image, (size, size))
            data = np.asarray(crop, dtype=np.float32)[None] / (127.5 if size == 128 else 255.0)
            if size == 128:
                data -= 1.0
            sessions = [ort.InferenceSession(str(directory / (name + ".onnx")), options, providers=["CPUExecutionProvider"]) for directory in (fresh, reference)]
            outputs = [session.run(None, {session.get_inputs()[0].name: data}) for session in sessions]
            if len(outputs[0]) != len(outputs[1]):
                raise RuntimeError("Output contract changed: " + name)
            for a, b in zip(*outputs):
                np.testing.assert_allclose(a, b, rtol=1e-4, atol=1e-3, err_msg=name)
            print("Numerical equivalence passed:", name)


if __name__ == "__main__":
    main()
