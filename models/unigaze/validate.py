"""GPU development proof: full frontend/layout + FP16 vs original FP32 weights.

Uses only the hash-locked public portrait and synthetic dim/shift variants; no
camera, user recordings or images are written. This is not gaze accuracy/G2.
Run with the installed environment: python validate.py --root <install-dir>
"""
import argparse
import contextlib
import json
from pathlib import Path
import sys

from worker import Pipeline


def validate(root):
    pipeline = Pipeline(root)
    np, torch, cv2 = pipeline.np, pipeline.torch, pipeline.cv2
    from safetensors.torch import load_file
    inputs, outputs, rotations = [], [], []
    transform = pipeline.transform
    normalize = pipeline.normalize

    def capture(image):
        result = transform(image)
        inputs.append(result.clone())
        return result

    def check_layout(image, *args, **kwargs):
        value = normalize(image, *args, **kwargs)
        # Official video path: BGR crop -> official warp -> RGB conversion.
        # LUCAS receives RGB, and must be exactly equivalent without a second swap.
        bgr = normalize(image[:, :, ::-1].copy(), *args, **kwargs)
        if not np.array_equal(value[0], bgr[0][:, :, ::-1]):
            raise RuntimeError("RGB/BGR normalization parity failed")
        rotations.append(value[1].copy())
        return value

    pipeline.transform = capture
    pipeline.normalize = check_layout
    hook = pipeline.model.register_forward_hook(
        lambda model, args, output: outputs.append(output["pred_gaze"][0].float().cpu().numpy()))
    image = pipeline.fixture()
    variants = [image, (image.astype(float)*0.55).astype(np.uint8),
                cv2.warpAffine(image, np.float32([[1,0,40],[0,1,-15]]), (1280,720))]
    for variant in variants:
        pipeline.infer(variant)
        if pipeline.last_model_ms <= 0:
            raise RuntimeError("synthetic variant did not reach the gaze network")
    hook.remove()
    # Reload ORIGINAL FP32 values, not merely upcast rounded half weights.
    pipeline.model.float()
    pipeline.model.load_state_dict(load_file(str(root/"unigaze_h14_joint.safetensors")), strict=True)
    torch.cuda.synchronize()
    deltas = []
    def ray(angles, rotation):
        pitch, yaw = angles
        vector = np.array([np.cos(pitch)*np.sin(yaw), np.sin(pitch), np.cos(pitch)*np.cos(yaw)])
        vector = np.linalg.inv(rotation) @ vector
        return vector/np.linalg.norm(vector)
    with torch.inference_mode():
        for image_tensor, half, rotation in zip(inputs, outputs, rotations, strict=True):
            full = pipeline.model(image_tensor.unsqueeze(0).to(pipeline.device))["pred_gaze"][0].cpu().numpy()
            delta = float(np.degrees(np.arccos(np.clip(np.dot(ray(full, rotation), ray(half, rotation)), -1, 1))))
            deltas.append(delta)
    if len(deltas) != 3 or max(deltas) > 0.25:
        raise RuntimeError(f"FP16 reference parity exceeds 0.25 degree: {deltas}")
    return {"gpu": pipeline.gpu, "torch": torch.__version__, "cuda": torch.version.cuda,
            "cases": ["public portrait", "synthetic dim", "synthetic shift"],
            "rgb_layout_exact": True, "camera_ray_fp16_vs_fp32_degrees": deltas,
            "tolerance_degrees": 0.25, "g2_or_webcam_acceptance": False}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    with contextlib.redirect_stdout(sys.stderr):
        result = validate(args.root.resolve())
    print(json.dumps(result, indent=2, allow_nan=False))
