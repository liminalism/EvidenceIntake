# Retired Python parity tools

These scripts are not runtime backends for Evidence Intake. Live SigLIP and
Qwen inference is accepted only through the checksum-pinned `evidence-trt`
broker. The Python implementations remain temporarily as offline benchmark
oracles for TensorRT export parity and are not included in the Windows runtime
package.

`embed_cli.py` implements the former JSON CLI contract. It
opens a Hugging Face SigLIP/SigLIP 2 directory with `local_files_only=True` and
does not upload stills or queries.

The batch command loads the model once and reads paths from standard input:

```powershell
'{"paths":["D:\\stills\\scene-0001.jpg"]}' |
  python embed_cli.py --model D:\models\siglip2-base-patch16-384 batch
```

Use the same model identifier and preprocessing for indexing and querying.
The Rust CLI no longer accepts this process boundary.

## Scene-caption backend

`caption_cli.py` provides the corresponding load-once contract for an offline
Qwen-style image-text model. It captions each retained still independently so
an 8 GiB GPU does not hold several image contexts at once. The model generates
a natural visual description, but deterministic code retains only a bounded
visible-feature vocabulary and renders those terms into the stored navigation
text; all other free-form prose is discarded. A result is either that
visible-feature index or `null`; null is an explicit abstention and does not
fail the remaining overnight job.

```powershell
'{"paths":["D:\\stills\\scene-0001.jpg"]}' |
  python caption_cli.py `
    --model D:\models\Qwen2-VL-2B-Instruct-895c3a49 `
    --device cuda `
    --torch-dtype float16 `
    batch
```

The model directory must already be complete. Loading uses
`local_files_only=True` and `trust_remote_code=False`; no frame or prompt is
sent over the network. The Rust `describe` and `analyze` commands now require
an installed broker model id and immutable revision.

## Retrieval benchmark

`benchmark_cli.py` compares stored frame embeddings with human-labelled
original-timeline intervals. Copy `benchmark.example.json`, replace its source
identity and example intervals, and finish the labels before examining any
model ranking. Keep static visible-state queries, settings, low-light examples,
small or occluded objects, negative queries, and deliberately temporal/action
queries distinct.

```powershell
python benchmark_cli.py `
  --model D:\models\siglip2-base-patch16-384 `
  --scenes D:\benchmark\scenes.json `
  --embeddings D:\benchmark\embeddings.json `
  --manifest D:\benchmark\labels.json `
  --output D:\benchmark\siglip2-base-384-report.json
```

The report contains two separate measurements:

- sampling opportunity: whether any retained still fell inside each labelled
  interval; and
- retrieval recall: whether a frame in that interval appeared within result
  budgets such as 5, 10, or 25.

Temporal/action and negative queries remain in the report for failure analysis
but are excluded from static-frame recall. Candidate ranks and original
timeline positions are recorded; cosine values are not. The report also hashes
the local weights and processor assets, times batched text queries, and records
peak CUDA allocation so two candidate models can be compared reproducibly.
