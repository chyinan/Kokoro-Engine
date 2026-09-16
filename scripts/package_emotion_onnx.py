#!/usr/bin/env python3
"""
Kokoro-Engine Chinese-Emotion-Small Model Packaging & Distribution Tool

Features:
- Validates model components (config.json, tokenizer.json, model.onnx / model.int8.onnx, etc.)
- Packs deterministic ZIP archives for distribution:
  1. chinese-emotion-small-onnx-int8.zip (Recommended: ~255MB, 2.5x faster CPU inference)
  2. chinese-emotion-small-onnx.zip (Full FP32: ~803MB)
- Generates manifest.json with SHA-256 checksums and sizes for release verification
"""

import os
import sys
import json
import zipfile
import hashlib
import argparse

REQUIRED_COMPANION_FILES = [
    "config.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "special_tokens_map.json",
]

def calculate_sha256(filepath: str) -> str:
    h = hashlib.sha256()
    with open(filepath, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()

def package_zip(source_dir: str, output_zip: str, model_filename: str) -> dict:
    model_src = os.path.join(source_dir, model_filename)
    if not os.path.exists(model_src):
        raise FileNotFoundError(f"Missing model file: {model_src}")

    for comp in REQUIRED_COMPANION_FILES:
        comp_src = os.path.join(source_dir, comp)
        if not os.path.exists(comp_src):
            raise FileNotFoundError(f"Missing companion file: {comp_src}")

    print(f"[*] Packaging {output_zip} using {model_filename}...")
    with zipfile.ZipFile(output_zip, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        for comp in REQUIRED_COMPANION_FILES:
            comp_path = os.path.join(source_dir, comp)
            z.write(comp_path, arcname=comp)
        # Always package into archive as "model.onnx" so clients have a canonical entrypoint
        z.write(model_src, arcname="model.onnx")

    size_bytes = os.path.getsize(output_zip)
    sha256_hash = calculate_sha256(output_zip)
    print(f"[+] Successfully created {os.path.basename(output_zip)} ({size_bytes} bytes, SHA256: {sha256_hash})")

    return {
        "filename": os.path.basename(output_zip),
        "size_bytes": size_bytes,
        "sha256": sha256_hash,
    }

def main():
    parser = argparse.ArgumentParser(description="Package Chinese-Emotion-Small ONNX models for Kokoro-Engine")
    parser.add_argument(
        "--source-dir",
        default=os.path.join(os.path.dirname(__file__), "..", "scratch", "emotion_onnx_export"),
        help="Directory containing exported ONNX and tokenizer files",
    )
    parser.add_argument(
        "--output-dir",
        default=os.path.join(os.path.dirname(__file__), "..", "scratch", "emotion_onnx_export"),
        help="Target directory for output zip archives and manifest.json",
    )
    args = parser.parse_args()

    source_dir = os.path.abspath(args.source_dir)
    output_dir = os.path.abspath(args.output_dir)
    os.makedirs(output_dir, exist_ok=True)

    print(f"Packaging models from {source_dir} to {output_dir}")

    manifest = {
        "model_id": "Johnson8187/Chinese-Emotion-Small",
        "format": "onnx",
        "labels": [
            "neutral", "caring", "happy", "angry",
            "sad", "questioning", "surprised", "disgusted"
        ],
        "artifacts": {},
    }

    # 1. Package INT8 (recommended default)
    int8_model = "model.int8.onnx"
    if os.path.exists(os.path.join(source_dir, int8_model)):
        int8_zip = os.path.join(output_dir, "chinese-emotion-small-onnx-int8.zip")
        manifest["artifacts"]["int8"] = package_zip(source_dir, int8_zip, int8_model)

    # 2. Package FP32 (full precision)
    fp32_model = "model.onnx"
    if os.path.exists(os.path.join(source_dir, fp32_model)):
        fp32_zip = os.path.join(output_dir, "chinese-emotion-small-onnx.zip")
        manifest["artifacts"]["fp32"] = package_zip(source_dir, fp32_zip, fp32_model)

    manifest_path = os.path.join(output_dir, "manifest.json")
    with open(manifest_path, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2, ensure_ascii=False)
    print(f"[+] Generated manifest at {manifest_path}")

    sha256_txt_path = os.path.join(output_dir, "sha256.txt")
    with open(sha256_txt_path, "w", encoding="utf-8") as f:
        for variant, art in manifest["artifacts"].items():
            f.write(f"{art['sha256']}  {art['filename']}\n")
    print(f"[+] Generated sha256.txt at {sha256_txt_path}")

if __name__ == "__main__":
    main()
