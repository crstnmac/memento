# Spike: Moondream Parakeet Redux vs Whisper Tiny (2026-10-06)

Question: can Memento use Moondream's Parakeet Redux for meeting/voice-note
transcription, and is it better than the Whisper Tiny it ships today?

## Setup

- Machine: Apple M2, 8 cores, 16 GB. **Not idle**: WindowServer, Claude and Chrome
  were using roughly 2–3 cores throughout, which hurts multi-threaded CPU runtimes.
- Redux runtime: [`mudler/parakeet.cpp`](https://github.com/mudler/parakeet.cpp) master
  (`9a28a3c`), CPU build, `redux-packed.gguf` (213,319,296 bytes, SHA-256 matches the
  project manifest). Moondream's own runtime (Photon) is Python-only and was not tested.
- Baseline: `ggml-tiny.en.bin` through `transcribe-rs` with `whisper-metal`, exactly as
  Memento runs it (release build).
- Audio: a 102.8 s, 296-word meeting script read by three macOS TTS voices, plus two
  noisy copies at 10 dB SNR (steady room noise; overlapping-talker babble made from the
  speech itself). **Synthetic speech**: it is easier than real recordings and 1 word is
  0.34 % WER, so treat small gaps as noise.
- Real human speech: two LibriSpeech clips (7 s, 24 s) shipped in the parakeet.cpp
  fixtures, compared qualitatively (the text is a well-known sample).

## Results

Word error rate (digits/ordinals normalised so "500" = "five hundred"):

| Audio | Whisper Tiny.en | Redux |
|---|---|---|
| clean | 0.0 % | 1.4 % |
| steady noise 10 dB | 1.7 % | 2.4 % |
| babble 10 dB | **8.8 %** | **2.0 %** |

Real speech (LibriSpeech, 24 s): Redux transcribed every word correctly. Whisper Tiny
made two distinct errors ("observe" for "observed" twice, "very likely old portrait" for
"very like the old portrait").

Speed and memory (median of 3 runs, 102.8 s of audio):

| | Whisper Tiny (Metal) | Redux (CPU) |
|---|---|---|
| wall time | 1.6–2.1 s (≈ 50–65× real time) | 10.8–18 s (≈ 4–10× real time) |
| peak memory | ≈ 270 MB | ≈ 1.4 GB |
| CPU time | small (GPU) | 48–119 s depending on threads |
| model file | 78 MB | 213 MB |

Redux thread sweep on the clean file (busy machine): 2 threads 3.9×, 4 threads 5.7×,
6 threads 4.9×, 8 threads 5.2× real time. More threads did not help.

Moondream's published figure for Redux on an M2 (38× on CPU) was **not reproduced** with
parakeet.cpp; that figure is for their Photon runtime. A quiet machine reached 9.5× once.

## Findings

- Accuracy: Redux is clearly better on real speech and in overlapping-talker noise;
  on clean synthetic speech both are near-perfect. This contradicts Moondream's note that
  Redux degrades in noise relative to the original Parakeet — that is a comparison with
  full-precision Parakeet, not with Whisper Tiny.
- Cost: roughly 5–10× slower than today and ~5× the memory, and it keeps the CPU busy for
  the whole job. A one-hour meeting would take on the order of 6–12 minutes.
- Redux is CPU-only and offline-only in parakeet.cpp (no Metal path, no streaming).

## Integration notes

- parakeet.cpp has a flat C API (ABI v10) and a `parakeet-cli` (1 MB binary linking
  ggml + Accelerate). Both Memento (via whisper-rs-sys) and parakeet.cpp bundle their
  own copy of ggml, so linking it statically into the app risks duplicate `ggml_*`
  symbols.
- Recommended: ship `parakeet-cli` as a Tauri sidecar (`externalBin`). That isolates the
  duplicate-ggml problem, returns the ~1.4 GB to the OS when each job ends, and survives
  `panic = "abort"`. It must be signed and notarized with the app.
- Offer it as an optional "higher accuracy" download next to the Whisper models, run after
  the recording stops (already how Memento transcribes), and keep Whisper Tiny as default.

## Not tested

Parakeet Ultra (942 MB q8_0, Metal-capable), VAD segmentation (`--vad`), non-English audio,
hour-long recordings, real laptop-microphone meeting audio, and Moondream's Photon runtime.
