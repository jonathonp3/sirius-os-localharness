# Models

Local models run through `llama-server` on `127.0.0.1:8080`.

## Installed models

Models live in:

```text
~/.local/share/llama.cpp/models
```

The active selection is:

```text
~/.config/local-harness/selected-model.json
```

Switch models from the **Models** panel. Switching restarts the llama user service, waits for `/health`, and rolls back if the new model fails to load.

## Hugging Face downloads

The Models panel can search Hugging Face GGUF repositories, list files, and install one after confirming repo, file, and size.

Downloads:

- run in the background with progress
- can be cancelled
- resume from `.part` files
- verify SHA-256 when available
- reject non-GGUF files

For gated repositories, set `HF_TOKEN` on the host or place a token at `~/.cache/huggingface/token`.

## Curated models

Featured entries live in `src/model-catalog.json`. Each entry records:

```text
repo, file, publisher, quant, size, license, capabilities, VRAM guidance
```

A curated entry may also declare an `mmproj` and `installable: false`.

## Paired mmproj files

Multimodal models need both the model and a projector. For LFM 3 Omni:

```text
LFM3-Omni-2B-Q4_K_M.gguf
mmproj-LFM3-Omni-2B-Q8_0.gguf
```

The installer downloads both and writes a sidecar:

```text
LFM3-Omni-2B-Q4_K_M.gguf.mmproj
```

On selection, `selected-model.json` records the projector, and the startup wrapper passes:

```text
--mmproj <path>
```

## LFM 3 Omni

LFM 3 Omni requires a llama.cpp build containing PR #31257 (merged 2026-10-27).

It supports text, image, audio, and video input. Audio input is experimental. Video requires `ffmpeg` on `PATH`.

The local build must be rebuilt after updating llama.cpp:

```bash
cd ~/.local/share/llama.cpp
git fetch origin
git pull --ff-only
cmake -S . -B build-lfm -DCMAKE_BUILD_TYPE=Release -DGGML_VULKAN=ON
cmake --build build-lfm -j16
```

Then replace the runtime:

```bash
mv build/bin build-old-bin
cp -a build-lfm/bin build/bin
systemctl --user restart llama-server
```

## Model sizing

```text
7B–8B Q4    ≈ 4.5–5.5 GB    fits 8 GB with room for context
14B Q4      ≈ 8–9 GB        needs partial offload
30B MoE     ≈ 15–17 GB      partial offload; slower
2B Omni     ≈ 1–2 GB        fits comfortably
```

Partial offload improves on CPU-only execution but is slower than fitting a smaller model fully on the GPU.

## Related

- [Providers](providers.md)
- [Architecture](architecture.md)
- [Troubleshooting](troubleshooting.md)
