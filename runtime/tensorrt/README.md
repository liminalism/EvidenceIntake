# Evidence TensorRT runtime

`evidence-trt-broker` is the single local inference boundary used by the
document, audio, and video adapters. On Windows its namespaced endpoint is a named pipe. The
broker verifies every installed model pack before serving, supervises one
native worker process per resident model, and evicts least-recently-used models
to stay within the configured VRAM budget. It never selects another backend
after an error. A pack's `runtime` names its worker family: `tensor_rt`,
`tensor_rt_llm`, or `native_cpu` for a worker that owns no GPU engine at all
(the speech pack). Every family is checksummed, revision-pinned, probed and
admitted the same way; `estimated_vram_bytes` is simply resident memory for a
CPU pack.

Each worker speaks the versioned `evidence.trt` framed protocol over standard
input/output. A frame is a little-endian `u32` JSON length, a little-endian
`u64` payload length, the JSON request envelope, and the binary payload. A
worker must perform a real engine load and inference probe before answering a
`load` request successfully. Inference operations are typed: page OCR, image
and text embedding, image detection, audio transcription, and bounded image
captioning.

## Model packs

Every direct child of the models directory is one pack with a `manifest.json`:

```json
{
  "schema_version": 1,
  "id": "siglip2-base-patch16-384",
  "revision": "immutable-export-revision",
  "provider": "Google",
  "license": "Apache-2.0",
  "runtime": "tensor_rt",
  "worker": "bin/siglip-worker.exe",
  "worker_args": [],
  "operations": ["embed_image", "embed_text"],
  "estimated_vram_bytes": 2147483648,
  "artifacts": [
    {
      "role": "worker",
      "path": "bin/siglip-worker.exe",
      "sha256": "64-lowercase-hex-digits"
    },
    {
      "role": "engine",
      "path": "models/image.engine",
      "sha256": "64-lowercase-hex-digits"
    }
  ]
}
```

The worker and every engine/tokenizer/dictionary asset must be pack-relative
and checksummed. Absolute paths and parent-directory traversal are rejected.
The caller supplies the expected revision on every inference; the broker
rejects a request if it differs from the verified manifest.

The intended installed packs are:

- TurboOCR/PP-OCR for `page_ocr`;
- SigLIP for `embed_image` and `embed_text`;
- YOLO for `detect_image`;
- Phonon-2 for `transcribe_audio` (default; a Whisper pack with the same
  typed contract may be installed beside it);
- Qwen-VL through TensorRT-LLM for `caption_image`.

## Speech: the Phonon-2 pack

`FermionResearch/Phonon-2` is a five-value quantization of NVIDIA's
`parakeet-tdt-0.6b-v3`: 164 MB, English only, word timestamps, no
diarization, no confidence scores, CC-BY-4.0 weights. It averages 5.21 % word
error over the seven Open ASR Leaderboard sets against 6.58 % for Whisper
large-v3-turbo at a tenth of the size, and transcribes an hour of audio in
well under a minute on an ordinary CPU, so it is the default speech pack and
needs no GPU.

`evidence-trt-phonon-worker` serves it. The worker bridges the framed
protocol to one `fermion transcribe <model> <wav> --json` call per request,
pins the engine's cache inside the pack (`FERMION_CACHE_DIR`) with network
lookups forbidden (`HF_HUB_OFFLINE=1`), refuses a non-English language hint
with `unsupported_language`, refuses a transcript the engine marks
`truncated`, and returns every segment with `speaker: None` and
`confidence: None` because the engine reports neither. The load probe decodes
one second of audio end to end before the broker admits the pack.

Build the pack from the Hugging Face download and the `fermion` launcher of a
`pip install fermion-research` environment (or any executable that behaves
like `fermion transcribe MODEL AUDIO --json`, such as a Docker wrapper):

```sh
hf download FermionResearch/Phonon-2 --local-dir Phonon-2
evidence-trt-phonon-worker pack \
  --weights Phonon-2 \
  --engine /path/to/venv/bin/fermion \
  --out models/phonon-2
```

`pack` verifies `phonon-2.bps.tar.zst` against the published SHA-256
(`98125795b6dd…`), copies the weights, `config.json`,
`packed_manifest.json`, the licence files, the launcher and the worker into
the pack, uses the archive hash as the manifest `revision`, writes
`manifest.json` with `"runtime": "native_cpu"`, and re-reads the result the
way the broker will. Pass `--weights-sha256` only to pin a different
published revision on purpose, and `--threads N` to fix the engine's CPU
thread count. The resulting directory is handed to
`package_windows_tensorrt.ps1` through `-ModelPacks` like any other pack.

## Windows package

First build the self-contained Lege OCR TensorRT payload with its own
`package_windows_tensorrt.ps1`. Evidence Intake converts that payload into the
checksummed `turbo-ocr` broker pack automatically; pass only the other exported
model packs:

```powershell
.\scripts\package_windows_tensorrt.ps1 `
  -LegeOcrPayload D:\packages\lege-ocr `
  -ModelPacks D:\models\siglip,D:\models\yolo,D:\models\phonon-2,D:\models\qwen `
  -TensorRTVersion 10.13 `
  -TensorRTLLMVersion 1.1 `
  -Gpu 'NVIDIA GeForce RTX 4060 Laptop GPU'
```

Staging fails if either doctor fails. The Lege doctor runs real OCR inference;
the Evidence doctor verifies every manifest and requires every worker's real
load probe. `run_windows_tensorrt.ps1` starts the packaged broker. No Python
runtime or network model lookup is included. `run_windows_document.ps1`
invokes the packaged document adapter and points Lege at that same `turbo-ocr`
pack, avoiding a second copy of TensorRT, OpenCV, and the OCR graphs.
