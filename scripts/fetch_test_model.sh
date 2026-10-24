#!/usr/bin/env bash
# Downloads Parakeet TDT 0.6B v2 int8 into a path with a space and "é" (exercises path handling)
# and writes a manifest.json matching the bundled catalog entry. Not committed; ~660 MB.
# Usage: scripts/fetch_test_model.sh [dest]   then:  UTTR_MODEL_DIR="<dest>" cargo test ... -- --ignored
set -euo pipefail
DEST="${1:-$HOME/Library/Caches/uttr Test é/parakeet-v2}"
BASE="https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/main"
mkdir -p "$DEST"
for f in encoder.int8.onnx decoder.int8.onnx joiner.int8.onnx tokens.txt; do
  [ -s "$DEST/$f" ] && [ ! -e "$DEST/$f.part" ] && continue
  curl -fL --retry 3 -C - -o "$DEST/$f" "$BASE/$f"
done
HERE="$(cd "$(dirname "$0")/.." && pwd)"
python3 - "$HERE/src-tauri/resources/catalog.json" "$DEST/manifest.json" <<'PY'
import json, sys
c = json.load(open(sys.argv[1]))
e = next(m for m in c["models"] if m["id"] == "parakeet-v2")
json.dump(e, open(sys.argv[2], "w"), indent=1)
PY
echo "model ready: $DEST"
