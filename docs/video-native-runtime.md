# Native visual-embedding runtime: benchmark first

The current Python/PyTorch backend is the reference implementation for model
evaluation. It is not a commitment to ship Python or PyTorch. If a visual
encoder proves useful on human-labelled body-camera retrieval tasks, a
model-specific Rust runtime is a credible follow-up.

## What `lege-gpu` demonstrates

The `lege-gpu` source at sibling-repository commit
`3f520cc03211e48adaa77c1da9bc639c53d2d351` is not able to run the downloaded
SigLIP 2 checkpoint directly. Its public ONNX session is intentionally narrow:
it accepts registered document-vision image inputs, exposes one `f32` runtime
tensor, implements a convolution-oriented operator set, and currently compiles
the generic graph again for each `run_gpu` call.

It is nevertheless a useful design and implementation reference. In
particular, it already demonstrates:

- deterministic discrete-GPU selection with explicit adapter overrides and
  device-loss handling;
- one process-wide `wgpu` device, immutable constant and pipeline sharing,
  resident activation buffers, buffer lifetime planning, and bounded readback;
- ONNX inspection, constant folding, static shape inference, explicit operator
  lowering, and a CPU reference executor used as a correctness oracle;
- GPU timestamp profiling and sibling sessions for controlled concurrency; and
- working Windows DX12 selection of this workstation's NVIDIA RTX 4060 Laptop
  GPU, confirmed by its hardware policy test.

Those are the difficult runtime disciplines we would want to preserve. The
SigLIP-specific math is a new layer on top of them.

## What a SigLIP runtime would need

The first native implementation should be deliberately model-specific, not a
new general-purpose inference framework. It would need:

1. Separate fixed contracts for the vision and text encoders, including
   `f32`/`f16` pixels and integer token IDs or attention masks.
2. Exact image preprocessing and Gemma SentencePiece tokenization parity with
   the pinned Python processor.
3. Transformer operations absent from the present `lege-gpu` bridge, including
   token gathering, LayerNorm, GELU, attention reshaping, batched matrix
   multiplication, masking, and final normalization.
4. A persistent compiled session that loads weights and creates pipelines once,
   then embeds many still batches and many later text queries.
5. `f16` weight/compute support where it is both available and retrieval-stable,
   with an `f32` reference path for differential tests.
6. Layer checkpoints and final golden embeddings exported by the Python oracle,
   plus end-to-end comparison of retrieval ordering rather than only numeric
   tensor tolerances.

A frozen ONNX export can be useful as an interchange and inspection format, but
the production Rust path need not inherit `lege-gpu`'s generic ONNX restrictions.
For one selected checkpoint, an offline converter can emit a small, validated
model manifest and packed weights for a fixed Rust graph. This avoids spending
the project on ONNX features the chosen model never uses.

## Reuse boundary

If the native work is approved, first extract or generalize only the proven
runtime pieces: adapter policy, shared device, compiled pipelines, resident
memory planning, profiling, and CPU/GPU differential-test structure. Do not
make the evidence application depend directly on a sibling working tree, and
do not widen the document-oriented `OnnxSession` until a reusable API is clear.
The evidence adapter should continue to depend on its existing
`EmbeddingBackend` boundary so Python and Rust implementations can be compared
against the same benchmark.

`lege-gpu` is MIT-licensed and this project is AGPL-3.0-or-later, so its code can
be reused with attribution. Model weights, tokenizer assets, and their exact
revision still need their own pinned provenance and licence record.

## Go/no-go gate

Native runtime work starts only after all of these are true:

- a human-authored interval benchmark shows that the chosen encoder retrieves
  materially useful moments within a realistic reviewer result budget;
- sampling opportunity and encoder retrieval recall have been measured
  separately, including low-light and small/occluded-object failures;
- one exact checkpoint, preprocessing contract, vector dimension, and result
  selection policy have been pinned; and
- Python deployment or throughput is demonstrably the remaining problem.

If the model is not useful, a Rust implementation would only make the wrong
answer arrive faster. Until this gate is met, Python remains the correctness
oracle and the fastest way to compare models.
