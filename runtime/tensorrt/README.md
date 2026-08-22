# Evidence TensorRT runtime

`evidence-trt-broker` is the single local inference boundary used by the
document, audio, and video adapters. On Windows its namespaced endpoint is a named pipe. The
broker verifies every installed model pack before serving, supervises one
native worker process per resident model, and evicts least-recently-used models
to stay within the configured VRAM budget. It never selects another backend
after an error.

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
- Whisper for `transcribe_audio`;
- Qwen-VL through TensorRT-LLM for `caption_image`.

## Windows package

First build the self-contained Lege OCR TensorRT payload with its own
`package_windows_tensorrt.ps1`. Evidence Intake converts that payload into the
checksummed `turbo-ocr` broker pack automatically; pass only the other exported
model packs:

```powershell
.\scripts\package_windows_tensorrt.ps1 `
  -LegeOcrPayload D:\packages\lege-ocr `
  -ModelPacks D:\models\siglip,D:\models\yolo,D:\models\whisper,D:\models\qwen `
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
