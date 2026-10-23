#!/usr/bin/env python3
"""Dev-only: regenerate src-tauri/resources/catalog.json.

Usage: python3 scripts/build_catalog.py > src-tauri/resources/catalog.json

Pins each repo to its current commit sha and resolves per-file size + sha256
from the HF API (`?blobs=true`, `lfs.sha256`). Non-LFS files (tokens.txt) are
downloaded and hashed locally. Stdlib only.
"""
import hashlib
import json
import sys
import urllib.request

API = "https://huggingface.co/api/models/csukuangfj/{repo}?blobs=true"
RAW = "https://huggingface.co/csukuangfj/{repo}/resolve/{rev}/{path}"

V3_LANGS = "bg hr cs da nl en et fi fr de el hu it lv lt mt pl pt ro sk sl es sv ru uk".split()
WHISPER_TURBO_LANGS = "en zh de es ru ko fr ja pt tr pl ca nl ar sv it id hi fi vi he uk el ms cs ro da hu ta no th ur hr bg lt".split()

MOONSHINE = ["preprocess.onnx", "encode.int8.onnx", "uncached_decode.int8.onnx",
             "cached_decode.int8.onnx", "tokens.txt"]
TRANSDUCER = ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"]


def whisper(p):
    return [f"{p}-encoder.int8.onnx", f"{p}-decoder.int8.onnx", f"{p}-tokens.txt"]


MODELS = [
    dict(id="parakeet-v2", name="Parakeet TDT 0.6B v2 (int8)", repo="sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
         engine="transducer", files=TRANSDUCER, languages=["en"], min_ram_gb=4,
         notes="Default. Best English accuracy and speed."),
    dict(id="parakeet-v3", name="Parakeet TDT 0.6B v3 (int8)", repo="sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
         engine="transducer", files=TRANSDUCER, languages=V3_LANGS, min_ram_gb=4,
         notes="25 European languages."),
    dict(id="moonshine-base", name="Moonshine Base (int8)", repo="sherpa-onnx-moonshine-base-en-int8",
         engine="moonshine", files=MOONSHINE, languages=["en"], min_ram_gb=2,
         notes="Small and fast; good for low-RAM machines."),
    dict(id="whisper-tiny-en", name="Whisper tiny.en (int8)", repo="sherpa-onnx-whisper-tiny.en",
         engine="whisper", files=whisper("tiny.en"), languages=["en"], min_ram_gb=2,
         notes="Smallest download; lowest accuracy."),
    dict(id="whisper-small-en", name="Whisper small.en (int8)", repo="sherpa-onnx-whisper-small.en",
         engine="whisper", files=whisper("small.en"), languages=["en"], min_ram_gb=3,
         notes="Balanced English model."),
    dict(id="sensevoice-small", name="SenseVoice small (int8)", repo="sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
         engine="sensevoice", files=["model.int8.onnx", "tokens.txt"], languages=["zh", "en", "ja", "ko", "yue"],
         min_ram_gb=2, notes="Chinese, English, Japanese, Korean, Cantonese."),
    dict(id="whisper-turbo", name="Whisper large-v3-turbo (int8)", repo="sherpa-onnx-whisper-turbo",
         engine="whisper", files=whisper("turbo"), languages=WHISPER_TURBO_LANGS, min_ram_gb=6,
         notes="35 languages (a subset of Whisper's 99). Large download."),
]


def get(url):
    with urllib.request.urlopen(url, timeout=120) as r:
        return r.read()


def main():
    out = []
    for m in MODELS:
        info = json.loads(get(API.format(repo=m["repo"])))
        rev = info["sha"]
        sib = {s["rfilename"]: s for s in info["siblings"]}
        files = []
        for path in m["files"]:
            s = sib[path]  # KeyError if the repo layout changed
            lfs = s.get("lfs")
            if lfs:
                sha = lfs["sha256"]
            else:
                sha = hashlib.sha256(get(RAW.format(repo=m["repo"], rev=rev, path=path))).hexdigest()
            files.append({"path": path, "size": s["size"], "sha256": sha})
        out.append({
            "id": m["id"], "name": m["name"], "hf_repo": "csukuangfj/" + m["repo"], "revision": rev,
            "files": files, "engine": m["engine"], "languages": m["languages"],
            "min_ram_gb": m["min_ram_gb"], "notes": m["notes"],
        })
        print("done", m["id"], file=sys.stderr)
    json.dump({"models": out}, sys.stdout, indent=2)
    print()


main()
