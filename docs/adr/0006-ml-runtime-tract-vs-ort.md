# 0006. ML inference runtime: tract vs ort

Date: 2026-11-13 · Status: proposed (decided by benchmark in M8)

## Context

The de-smack model runs per 10 ms hop with ≤ 1 ms inference budget on the audio path or an
audio-workgroup worker thread.

## Decision

Pending. Prefer `tract-onnx` (pure Rust, no dylib, used by DeepFilterNet) unless `ort` is
substantially faster at p99 on the exported model. Benchmark both in M8 on the real model.

## Alternatives considered

- `tract-onnx` 0.23.x, `ort` (ONNX Runtime bindings), hand-written inference for tiny GRUs.

## Consequences (incl. how we'd know it was wrong)

- To be filled in with p50/p99 numbers and bundle size impact.
