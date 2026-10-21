"""CPU-only behavioral tests for download integrity and the child wire protocol.
Run: python -m unittest discover -s models/unigaze -p 'test_*.py'
No ML packages, network requests, camera or GPU are used.
"""
import hashlib
import io
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

import prepare
import worker


class Fragmented(io.BytesIO):
    def readinto(self, buffer):
        return super().readinto(memoryview(buffer)[:3])


class FakePipeline:
    gpu = "TEST, not a GPU"
    def __init__(self, root):
        self.np = self
    def frombuffer(self, data, dtype):
        self.data = data
        return self
    uint8 = 0
    def reshape(self, h, w, channels):
        assert len(self.data) == h*w*channels
        return self
    def infer(self, image):
        return {"features": None, "quality": 0.0, "face_found": False, "luminance": 0.5}
    def check(self):
        return {"inferences": 20, "provider": "test"}


class Protocol(unittest.TestCase):
    def exchange(self, data, error=False):
        output = io.StringIO()
        with patch.object(worker, "Pipeline", FakePipeline):
            if error:
                with self.assertRaises(SystemExit):
                    worker.serve(Path("unused"), Fragmented(data), output)
            else:
                worker.serve(Path("unused"), Fragmented(data), output)
        return [json.loads(line) for line in output.getvalue().splitlines()]

    def test_fragmented_frames_preserve_original_acquisition_time_and_unknown(self):
        data = worker.HEADER.pack(b"LUG1", 2, 2, 123456789) + bytes(range(12))
        ready, sample = self.exchange(data)
        self.assertTrue(ready["ready"])
        self.assertEqual(sample["timestamp_ns"], 123456789)
        self.assertIsNone(sample["features"])
        self.assertEqual(sample["quality"], 0)
        self.assertNotIn("pixels", sample)

    def test_check_is_a_bounded_command_without_frame_payload(self):
        ready, result = self.exchange(worker.HEADER.pack(b"LUG1", 0, 0, 0))
        self.assertEqual(result["inferences"], 20)

    def test_rejects_bad_magic_dimensions_truncation_and_repeated_timestamps(self):
        frame = worker.HEADER.pack(b"LUG1", 2, 2, 10) + bytes(12)
        for data in [b"L", worker.HEADER.pack(b"BAD!", 2, 2, 10),
                     worker.HEADER.pack(b"LUG1", 1921, 1080, 10),
                     worker.HEADER.pack(b"LUG1", 0, 2, 10),
                     frame[:-1], frame+frame]:
            self.assertIn("error", self.exchange(data, error=True)[-1])

    def test_missing_assets_fail_before_importing_gpu_libraries(self):
        with tempfile.TemporaryDirectory() as folder:
            with self.assertRaisesRegex(RuntimeError, "asset missing"):
                worker.verify_assets(Path(folder))


class Downloads(unittest.TestCase):
    def test_hash_checked_install_reuses_verified_file_and_rejects_corruption(self):
        with tempfile.TemporaryDirectory() as folder:
            base = Path(folder)
            source = base/"published.bin"
            source.write_bytes(b"published synthetic asset")
            asset = {"file": "model.bin", "url": source.as_uri(),
                     "sha256": hashlib.sha256(source.read_bytes()).hexdigest()}
            root = base/"install"
            prepare.prepare(root, {"assets": [asset]})
            self.assertEqual((root/"model.bin").read_bytes(), source.read_bytes())
            source.unlink()
            prepare.prepare(root, {"assets": [asset]})  # no download needed
            source.write_bytes(b"corrupt upstream")
            (root/"model.bin").write_bytes(b"corrupt local")
            with self.assertRaisesRegex(RuntimeError, "SHA-256 mismatch"):
                prepare.prepare(root, {"assets": [asset]})
            self.assertFalse((root/"model.bin.part").exists())
            self.assertEqual((root/"model.bin").read_bytes(), b"corrupt local")


if __name__ == "__main__":
    unittest.main()
