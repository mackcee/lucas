"""Private LUCAS child. Anonymous pipes only; no camera, sockets or runtime downloads.

Protocol v1: LE <4sIIQ> ('LUG1', width, height, original timestamp_ns),
then exactly width*height*3 RGB8 bytes. One request in flight. 0x0 runs the
bounded bundled-face diagnostic. Replies are bounded JSON lines, never pixels.
"""
import argparse
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import struct
import sys
import time
import zipfile

HEADER = struct.Struct("<4sIIQ")
MAX_PIXELS = 1920 * 1080


def read_exact(stream, size):
    data = bytearray(size)
    view = memoryview(data)
    offset = 0
    try:
        while offset < size:
            count = stream.readinto(view[offset:])
            if not count:
                if offset == 0:
                    return None
                raise ValueError("truncated frame packet")
            offset += count
        return data
    except BaseException:
        data[:] = b"\x00" * len(data)
        raise


def verify_assets(root):
    lock = json.loads(Path(__file__).with_name("assets.json").read_text())
    for asset in lock["assets"]:
        path = root / asset["file"]
        if not path.is_file():
            raise RuntimeError(f"UniGaze asset missing: {path.name}; run setup.ps1")
        with path.open("rb") as stream:
            actual = hashlib.file_digest(stream, "sha256").hexdigest()
        if actual != asset["sha256"]:
            raise RuntimeError(f"UniGaze SHA-256 mismatch: {path.name}; rerun setup.ps1")
    return lock


class Pipeline:
    def __init__(self, root):
        lock = verify_assets(root)
        # Scope all library caches to the install; never fetch anything here.
        os.environ["HF_HUB_OFFLINE"] = "1"
        os.environ["TORCH_HOME"] = str(root / "cache")
        os.environ["NUMBA_CACHE_DIR"] = str(root / "cache" / "numba")
        for key, directory in (("CUDA_CACHE_PATH", "cuda"), ("TMP", "tmp"),
                               ("TEMP", "tmp"), ("TMPDIR", "tmp")):
            path = root / "cache" / directory
            path.mkdir(parents=True, exist_ok=True)
            os.environ[key] = str(path)
        os.environ["CUBLAS_WORKSPACE_CONFIG"] = ":4096:8"
        import torch
        import numpy as np
        import cv2
        import face_alignment
        import face_alignment.api
        from safetensors.torch import load_file
        self.torch, self.np, self.cv2 = torch, np, cv2
        if not torch.cuda.is_available():
            raise RuntimeError("CUDA unavailable: install the NVIDIA driver and run setup.ps1 (no CPU fallback)")
        self.device = torch.device("cuda:0")
        torch.cuda.set_device(self.device)
        prop = torch.cuda.get_device_properties(self.device)
        if prop.major >= 10 and (torch.version.cuda is None or tuple(map(int, torch.version.cuda.split('.')[:2])) < (12, 8)):
            raise RuntimeError("Blackwell requires CUDA 12.8 or newer; rerun setup.ps1")
        self.gpu = prop.name
        # Bound allocator/workspace; never run cuDNN's expensive autotuning.
        torch.cuda.set_per_process_memory_fraction(min(1.0, 8 * 1024**3 / prop.total_memory), self.device)
        torch.set_num_threads(2)
        cv2.setNumThreads(1)
        torch.backends.cudnn.benchmark = False
        torch.backends.cuda.matmul.allow_tf32 = False
        torch.backends.cudnn.allow_tf32 = False
        prefix = f"UniGaze-{lock['revision']}"
        archive = str(root / "source.zip")
        sys.path[:0] = [archive + f"/{prefix}/unigaze_easy/src", archive + f"/{prefix}/unigaze"]
        from unigaze.loader import build_unigaze_model
        from gazelib.gaze.normalize import estimateHeadPose, normalize
        from gazelib.label_transform import get_face_center_by_nose
        from torchvision import transforms
        self.pose, self.normalize, self.center = estimateHeadPose, normalize, get_face_center_by_nose
        with zipfile.ZipFile(root / "source.zip") as source:
            self.template = np.loadtxt(io.BytesIO(source.read(f"{prefix}/unigaze/data/face_model.txt")))
        self.transform = transforms.Compose([
            transforms.ToPILImage(), transforms.ToTensor(),
            transforms.Normalize([0.485, 0.456, 0.406], [0.229, 0.224, 0.225]),
        ])
        self.model = build_unigaze_model("unigaze_h14_joint")
        self.model.load_state_dict(load_file(str(root / "unigaze_h14_joint.safetensors")), strict=True)
        self.model = self.model.eval().half().to(self.device)
        # The upstream initializer normally downloads. Replace that one loader
        # with an exact allowlist and supply SFD's verified local path explicitly.
        def local_fan(url, *args, **kwargs):
            if url != "https://www.adrianbulat.com/downloads/python-fan/2DFAN4-cd938726ad.zip":
                raise RuntimeError("unexpected FAN model request; runtime downloads are disabled")
            return str(root / "2DFAN4-cd938726ad.zip")
        face_alignment.api.load_file_from_url = local_fan
        self.frontend = face_alignment.FaceAlignment(
            face_alignment.LandmarksType.TWO_D, device="cuda:0", flip_input=False,
            face_detector="sfd", face_detector_kwargs={"path_to_detector": str(root / "s3fd-619a316812.pth")},
        )
        # FaceAlignment sets this True in its constructor, before any inference.
        torch.backends.cudnn.benchmark = False
        self.root = root
        self.last_model_ms = 0.0
        # Warm the full path (including Numba/CUDA kernels) before opening the
        # camera. First-use compilation must not consume calibration targets.
        fixture = self.fixture()
        for _ in range(3):
            self.infer(fixture)
            if self.last_model_ms <= 0:
                raise RuntimeError("bundled face did not reach UniGaze during startup")
        fixture.fill(0)

    def infer(self, rgb):
        np, cv2, torch = self.np, self.cv2, self.torch
        self.last_model_ms = 0.0
        h, w = rgb.shape[:2]
        luma = float(np.mean(rgb[::16, ::16] @ np.array([0.2126, 0.7152, 0.0722])) / 255)
        result = {"features": None, "quality": 0.0, "face_found": False, "luminance": luma}
        small = cv2.resize(rgb, None, fx=0.5, fy=0.5, interpolation=cv2.INTER_AREA)
        with torch.inference_mode():
            preds, scores, boxes = self.frontend.get_landmarks_from_image(
                small, return_landmark_score=True, return_bboxes=True)
        if preds is None or len(preds) != 1:
            return result  # multiple people are ambiguous, not an arbitrary gaze
        points = preds[0].astype(float) / 0.5
        result["face_found"] = True
        if not np.isfinite(points).all() or (points[:, 0] < 0).any() or (points[:, 0] >= w).any() or (points[:, 1] < 0).any() or (points[:, 1] >= h).any():
            return result
        # Match official predict_gaze_video.py's 2x crop, integer bbox and camera.
        x0, y0 = points.min(axis=0).astype(int)
        x1, y1 = points.max(axis=0).astype(int)
        cx, cy = (x0 + x1) // 2, (y0 + y1) // 2
        left, right = max(0, cx - (x1 - x0)), min(w, cx + (x1 - x0))
        top, bottom = max(0, cy - (y1 - y0)), min(h, cy + (y1 - y0))
        if right - left < 32 or bottom - top < 32:
            return result
        crop = rgb[top:bottom, left:right]
        landmarks = points - [left, top]
        ch, cw = crop.shape[:2]
        camera = np.array([[cw*4, 0, cw//2], [0, cw*4, ch//2], [0, 0, 1.0]], dtype=float)
        face = self.template[[20, 23, 26, 29, 15, 19]].reshape(6, 1, 3)
        selected = landmarks[[36, 39, 42, 45, 31, 35]].reshape(6, 1, 2)
        try:
            hr, ht = self.pose(selected, face, camera, np.zeros((1, 5)))
            rotation = cv2.Rodrigues(hr)[0]
            center, _ = self.center(rotation, ht, self.template)
            normalized, R, head, _, _, _ = self.normalize(
                crop, landmarks, 960, 600, (224, 224), center, hr, ht, camera)
        except cv2.error:
            return result
        pose = np.array([np.arcsin(np.clip(head[1, 2], -1, 1)), np.arctan2(head[0, 2], head[2, 2])])
        if not np.isfinite(center).all() or center[2, 0] <= 0 or np.linalg.norm(pose) > np.deg2rad(80):
            return result
        # Input was already RGB; do NOT repeat the BGR->RGB swap from the demo.
        image = self.transform(normalized).unsqueeze(0).half().to(self.device)
        torch.cuda.synchronize()
        start = time.perf_counter()
        with torch.inference_mode():
            angles = self.model(image)["pred_gaze"][0].float().cpu().numpy()
        self.last_model_ms = (time.perf_counter() - start) * 1000
        pitch, yaw = angles
        vector = np.array([np.cos(pitch)*np.sin(yaw), np.sin(pitch), np.cos(pitch)*np.cos(yaw)])
        vector = np.linalg.inv(R) @ vector
        vector /= np.linalg.norm(vector)
        # The reference uses +z here (the displayed outgoing arrow is negated).
        pitch, yaw = np.arcsin(np.clip(vector[1], -1, 1)), np.arctan2(vector[0], vector[2])
        def aperture(i):
            eye = points[i:i+6]
            width = np.linalg.norm(eye[0] - eye[3])
            return float((np.linalg.norm(eye[1]-eye[5]) + np.linalg.norm(eye[2]-eye[4])) / (2*max(width, 1e-6)))
        apertures = [aperture(36), aperture(42)]
        if not all(0.06 < a < 0.65 for a in apertures):
            return result
        quality = float(np.clip(np.min(scores[0][36:48]), 0, 1))
        quality *= min(1.0, np.linalg.norm(points[45]-points[36]) / 80)
        quality *= min(1.0, max(0.0, (min(apertures)-0.06)/0.12))
        values = [float(pitch), float(yaw), *apertures, *map(float, pose), float(np.arctan2(head[1, 0], head[0, 0])), *map(float, center[:, 0])]
        if np.isfinite(values).all() and quality > 0:
            result.update(features=values, quality=quality)
        return result

    def fixture(self):
        np, cv2 = self.np, self.cv2
        image = cv2.cvtColor(cv2.imread(str(self.root / "portrait.jpg")), cv2.COLOR_BGR2RGB)
        h, w = image.shape[:2]
        resized = cv2.resize(image, (int(w * 720/h), 720), interpolation=cv2.INTER_AREA)
        image = np.zeros((720, 1280, 3), np.uint8)
        x = (1280-resized.shape[1])//2
        image[:, x:x+resized.shape[1]] = resized
        return image

    def check(self):
        np, torch = self.np, self.torch
        image = self.fixture()
        latency, model_latency = [], []
        # Keep startup allocation peaks in the memory report.
        for i in range(23):
            torch.cuda.synchronize()
            start = time.perf_counter()
            self.infer(image)
            torch.cuda.synchronize()
            elapsed = (time.perf_counter()-start)*1000
            if self.last_model_ms <= 0:
                raise RuntimeError("diagnostic face did not reach UniGaze inference")
            if i >= 3:
                latency.append(elapsed)
                model_latency.append(self.last_model_ms)
        image.fill(0)
        return {"provider": "PyTorch CUDA (cuda:0), UniGaze-H FP16; SFD/FAN FP32",
                "gpu": self.gpu, "inferences": 20,
                "median_ms": float(np.median(latency)), "p95_ms": float(np.percentile(latency, 95)),
                "model_median_ms": float(np.median(model_latency)), "model_p95_ms": float(np.percentile(model_latency, 95)),
                "allocated_mib": torch.cuda.memory_allocated()/1024**2,
                "reserved_mib": torch.cuda.memory_reserved()/1024**2,
                "peak_mib": torch.cuda.max_memory_allocated()/1024**2}


def serve(root, input_stream, output):
    def reply(value):
        output.write(json.dumps(value, allow_nan=False, separators=(",", ":")) + "\n")
        output.flush()
    try:
        pipeline = Pipeline(root)
        reply({"ready": True, "provider": "PyTorch CUDA cuda:0 / UniGaze-H FP16", "gpu": pipeline.gpu})
        previous = None
        while True:
            header = read_exact(input_stream, HEADER.size)
            if header is None:
                return
            magic, w, h, timestamp = HEADER.unpack(header)
            if magic != b"LUG1":
                raise ValueError("invalid protocol magic")
            if w == h == timestamp == 0:
                reply(pipeline.check())
                continue
            if w == 0 or h == 0 or w*h > MAX_PIXELS or (previous is not None and timestamp <= previous):
                raise ValueError("invalid frame dimensions or acquisition timestamp")
            previous = timestamp
            data = read_exact(input_stream, w*h*3)
            if data is None:
                raise ValueError("missing frame payload")
            try:
                result = pipeline.infer(pipeline.np.frombuffer(data, dtype=pipeline.np.uint8).reshape(h, w, 3))
                result["timestamp_ns"] = timestamp
                reply(result)
            finally:
                data[:] = b"\x00" * len(data)
    except Exception as error:
        # No image/tensor reprs or tracebacks in the app protocol.
        reply({"error": f"{type(error).__name__}: {str(error)[:1000]}"})
        raise SystemExit(1) from None


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    # Third-party loaders sometimes print; stdout belongs exclusively to IPC.
    output = sys.stdout
    with contextlib.redirect_stdout(sys.stderr):
        serve(args.root.resolve(), sys.stdin.buffer, output)
