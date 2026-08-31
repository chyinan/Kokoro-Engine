#!/usr/bin/env bash
set -euo pipefail

target_root="${CARGO_TARGET_DIR:-src-tauri/target}"
sherpa_root="$target_root/sherpa-onnx-prebuilt"

# A missing cache is normal: sherpa-onnx-sys will download it during the build.
if [[ ! -d "$sherpa_root" ]]; then
  exit 0
fi

lib_dir="$(find "$sherpa_root" -type d -name lib -print -quit 2>/dev/null || true)"
if [[ -z "$lib_dir" ]]; then
  echo "Removing incomplete Sherpa-ONNX target cache (no lib directory found)."
  rm -rf "$target_root"
  exit 0
fi

if [[ "${RUNNER_OS:-}" == "Windows" ]]; then
  prefix=""
  extension="lib"
else
  prefix="lib"
  extension="a"
fi

required_libraries=(
  sherpa-onnx-c-api
  sherpa-onnx-core
  kaldi-decoder-core
  sherpa-onnx-kaldifst-core
  sherpa-onnx-fstfar
  sherpa-onnx-fst
  kaldi-native-fbank-core
  kissfft-float
  piper_phonemize
  espeak-ng
  ucd
  onnxruntime
  ssentencepiece_core
)

missing=()
for library in "${required_libraries[@]}"; do
  path="$lib_dir/${prefix}${library}.${extension}"
  if [[ ! -f "$path" ]]; then
    missing+=("$path")
  fi
done

if (( ${#missing[@]} > 0 )); then
  printf 'Removing incomplete Sherpa-ONNX target cache; missing libraries:\n'
  printf '  %s\n' "${missing[@]}"
  rm -rf "$target_root"
else
  echo "Sherpa-ONNX target cache is complete: $lib_dir"
fi
