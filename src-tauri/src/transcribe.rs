//! Native transcription via transcribe-rs (whisper.cpp + GGML whisper models).
//!
//! Owns the whole recording → transcript pipeline on the Rust side:
//!   1. GGML whisper-tiny model management (download + cache under app data),
//!   2. WAV decode, channel downmix, resample to 16 kHz mono (whisper's rate),
//!   3. optional mic + system mixdown per the `mixAudio` setting,
//!   4. inference with a cached, serialized WhisperEngine,
//!   5. persisting the transcript and emitting progress events so the UI can
//!      follow along even when the main window is closed.
//!
//! This replaces the old in-webview Transformers.js worker, which failed
//! silently, assumed 16 kHz input, and could not run when the window was shut.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use transcribe_rs::whisper_cpp::{WhisperEngine, WhisperInferenceParams};
use transcribe_rs::SpeechModel;

use crate::db;
use crate::state::MonitorState;

/// Whisper expects 16 kHz mono f32 samples in [-1, 1].
const TARGET_RATE: u32 = 16_000;

/// Canonical transcription models (whisper.cpp GGML builds, ~75 MB each).
pub const MODEL_TINY_EN: &str = "whisper-tiny.en";
pub const MODEL_TINY: &str = "whisper-tiny";
const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Guard so the same conversation is never transcribed twice concurrently.
static IN_FLIGHT: OnceLock<Mutex<Vec<i64>>> = OnceLock::new();

/// Cached engine so repeated transcriptions don't reload the model. Holding
/// the lock during inference also serializes transcriptions, which keeps
/// memory and CPU predictable.
struct CachedEngine {
    model_id: String,
    path: PathBuf,
    engine: WhisperEngine,
}
static ENGINE: OnceLock<Mutex<Option<CachedEngine>>> = OnceLock::new();

/// Serializes model downloads so onboarding and a first transcription can't
/// race into two partial downloads of the same file.
static DOWNLOAD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn in_flight() -> &'static Mutex<Vec<i64>> {
    IN_FLIGHT.get_or_init(|| Mutex::new(Vec::new()))
}

fn engine_cache() -> &'static Mutex<Option<CachedEngine>> {
    ENGINE.get_or_init(|| Mutex::new(None))
}

/// Map any historical model ID (including Transformers.js-era values) to a
/// canonical whisper.cpp model ID.
pub fn canonical_model_id(raw: &str) -> String {
    match raw.trim() {
        "" | "whisper-tiny.en" | "Xenova/whisper-tiny.en" | "Xenova/whisper-tiny.en-onnx" => {
            MODEL_TINY_EN.to_string()
        }
        "whisper-tiny" | "Xenova/whisper-tiny" => MODEL_TINY.to_string(),
        other => other.to_string(),
    }
}

fn ggml_file_name(model_id: &str) -> String {
    match model_id {
        MODEL_TINY_EN => "ggml-tiny.en.bin".to_string(),
        MODEL_TINY => "ggml-tiny.bin".to_string(),
        // Unknown IDs are treated as bare GGML file names inside the models
        // dir. Only the final path component is kept: the id can come from the
        // webview/settings, and "../memento.db" must not escape the directory
        // (delete_model would otherwise delete it).
        other => Path::new(other)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty() && *name != "." && *name != "..")
            .unwrap_or("invalid-model")
            .to_string(),
    }
}

fn download_url(model_id: &str) -> Option<String> {
    match model_id {
        MODEL_TINY_EN => Some(format!("{HF_BASE}/ggml-tiny.en.bin")),
        MODEL_TINY => Some(format!("{HF_BASE}/ggml-tiny.bin")),
        _ => None,
    }
}

fn models_dir() -> Result<PathBuf, String> {
    let dir = db::app_data_dir()?.join("models");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create models dir: {e}"))?;
    Ok(dir)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadProgress {
    pub model: String,
    pub received: u64,
    pub total: Option<u64>,
}

/// Public entry point for the explicit "download the model first" step in
/// onboarding. Returns immediately if the model is already on disk.
pub fn prepare_model(app: &AppHandle, model_id: &str) -> Result<PathBuf, String> {
    ensure_model(app, model_id)
}

/// Presence + on-disk size of a transcription model.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub ready: bool,
    pub bytes: u64,
}

pub fn model_status(model_id: &str) -> ModelStatus {
    let path = models_dir()
        .ok()
        .map(|dir| dir.join(ggml_file_name(model_id)));
    match path {
        Some(path) if path.exists() => ModelStatus {
            ready: true,
            bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
        },
        _ => ModelStatus {
            ready: false,
            bytes: 0,
        },
    }
}

/// Delete the GGML model file (and any partial download) and drop the cached
/// engine so the next transcription re-resolves from disk.
pub fn delete_model(model_id: &str) -> Result<(), String> {
    let dir = models_dir()?;
    let path = dir.join(ggml_file_name(model_id));
    let partial = path.with_extension("part");
    for candidate in [path, partial] {
        if candidate.exists() {
            std::fs::remove_file(&candidate)
                .map_err(|e| format!("could not delete model file: {e}"))?;
        }
    }
    if let Ok(mut guard) = engine_cache().lock() {
        if guard
            .as_ref()
            .map(|cached| cached.model_id == model_id)
            .unwrap_or(false)
        {
            *guard = None;
        }
    }
    Ok(())
}

/// Make sure the GGML model file exists locally, downloading it from the
/// whisper.cpp HuggingFace mirror on first use. Emits `model-download-progress`
/// events while downloading.
fn ensure_model(app: &AppHandle, model_id: &str) -> Result<PathBuf, String> {
    let dir = models_dir()?;
    let path = dir.join(ggml_file_name(model_id));
    if path.exists() {
        return Ok(path);
    }

    let lock = DOWNLOAD_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "model download lock poisoned")?;
    // Another thread may have finished the download while we waited.
    if path.exists() {
        drop(lock);
        return Ok(path);
    }

    let url = download_url(model_id).ok_or_else(|| {
        format!(
            "unknown transcription model \"{model_id}\" — pick whisper-tiny.en (English) or whisper-tiny (multilingual)"
        )
    })?;

    log::info!("transcribe: downloading model {model_id} from {url}");
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(15))
        // A stalled connection must fail, not hang forever holding the
        // download lock.
        .timeout_read(std::time::Duration::from_secs(30))
        .build();
    let response = agent
        .get(&url)
        .call()
        .map_err(|e| format!("model download failed: {e}"))?;
    // HF error pages come back as HTML with 200 on some CDNs; verify the type.
    let content_type = response.content_type().to_lowercase();
    if content_type.contains("text/html") {
        return Err(format!(
            "model download failed: the server returned an HTML error page for {url}. Check your internet connection and retry."
        ));
    }
    let total = response
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok());

    let tmp = path.with_extension("part");
    // Remove the partial file on any early return (error, truncation, HTML).
    struct PartialFile(PathBuf, bool);
    impl Drop for PartialFile {
        fn drop(&mut self) {
            if !self.1 {
                let _ = std::fs::remove_file(&self.0);
            }
        }
    }
    let mut partial = PartialFile(tmp.clone(), false);
    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(&tmp)
        .map_err(|e| format!("model download failed: {}: {e}", tmp.display()))?;
    let mut buffer = [0u8; 256 * 1024];
    let mut received: u64 = 0;
    let mut last_emitted: u64 = 0;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("model download failed: {e}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|e| format!("model download failed: {e}"))?;
        received += read as u64;
        // Emit roughly every 5% so the UI can show a moving indicator.
        let should_emit = match total {
            Some(t) if t > 0 => received - last_emitted >= t / 20,
            _ => received - last_emitted >= 8 * 1024 * 1024,
        };
        if should_emit {
            last_emitted = received;
            let _ = app.emit(
                "model-download-progress",
                &ModelDownloadProgress {
                    model: model_id.to_string(),
                    received,
                    total,
                },
            );
        }
    }
    file.sync_all().ok();
    drop(file);
    // Reject truncated downloads.
    if let Some(t) = total {
        if received != t {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!(
                "model download failed: incomplete download (got {received} of {t} bytes) — retry"
            ));
        }
    }
    if received < 1024 * 1024 {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "model download failed: file is suspiciously small ({received} bytes)"
        ));
    }
    // Reject HTML error pages saved as body.
    let mut head = [0u8; 2];
    let looks_html = std::fs::File::open(&tmp)
        .and_then(|mut f| f.read_exact(&mut head).map(|_| head == *b"<!"))
        .unwrap_or(false);
    if looks_html {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "model download failed: the server returned an HTML error page for {url}. Check your internet connection and retry."
        ));
    }
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("model download failed (finalize): {e}"))?;
    partial.1 = true;
    log::info!("transcribe: model {model_id} downloaded ({} bytes)", received);
    Ok(path)
}

/// Read a WAV file written by the recorder, downmix to mono, and return the
/// samples with their original rate.
pub(crate) fn load_wav_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let spec = reader.spec();
    let rate = spec.sample_rate;
    let channels = spec.channels.max(1) as usize;
    let mut interleaved: Vec<f32> = Vec::new();
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for sample in reader.samples::<f32>() {
                interleaved.push(sample.map_err(|e| format!("corrupt WAV {}: {e}", path.display()))?);
            }
        }
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample.saturating_sub(1)).max(1)) as f32;
            for sample in reader.samples::<i32>() {
                interleaved
                    .push(sample.map_err(|e| format!("corrupt WAV {}: {e}", path.display()))? as f32 / max);
            }
        }
    }
    let mut mono = Vec::with_capacity(interleaved.len() / channels + 1);
    for frame in interleaved.chunks(channels) {
        mono.push(frame.iter().sum::<f32>() / frame.len() as f32);
    }
    Ok((mono, rate))
}

/// Linear-interpolation resample. Adequate for speech at 48k → 16k and keeps
/// the dependency surface small.
fn resample_to_16k(input: &[f32], from_rate: u32) -> Vec<f32> {
    if input.is_empty() || from_rate == TARGET_RATE {
        return input.to_vec();
    }
    let ratio = f64::from(from_rate) / f64::from(TARGET_RATE);
    let out_len = ((input.len() as f64) / ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 * ratio;
        if ratio > 1.0 {
            // Downsampling: average the source window instead of picking two
            // neighbours, which aliases high frequencies into the speech band.
            // Bounds are derived so they cannot panic (release builds abort).
            let start = (src as usize).min(input.len() - 1);
            let end = ((src + ratio).ceil() as usize)
                .min(input.len())
                .max(start + 1);
            let window = &input[start..end];
            out.push(window.iter().sum::<f32>() / window.len() as f32);
        } else {
            let i0 = (src as usize).min(input.len() - 1);
            let i1 = (i0 + 1).min(input.len() - 1);
            let t = (src - i0 as f64) as f32;
            out.push(input[i0] * (1.0 - t) + input[i1] * t);
        }
    }
    out
}

/// Average two equal-rate mono tracks; where only one has audio, keep it.
fn mix_tracks(a: &[f32], b: &[f32]) -> Vec<f32> {
    let len = a.len().max(b.len());
    (0..len)
        .map(|i| match (a.get(i), b.get(i)) {
            (Some(x), Some(y)) => (x + y) * 0.5,
            (Some(x), None) | (None, Some(x)) => *x,
            _ => 0.0,
        })
        .collect()
}

/// Load every WAV track (mic first, system second), resample to 16 kHz mono,
/// and mix per the `mix_audio` preference. Paths are stored as "mic,system".
fn prepare_samples(paths: &[String], mix_audio: bool) -> Result<Vec<f32>, String> {
    let mut tracks: Vec<Vec<f32>> = Vec::new();
    for path in paths {
        if let Ok((mono, rate)) = load_wav_mono(Path::new(path)) {
            if !mono.is_empty() {
                tracks.push(resample_to_16k(&mono, rate));
            }
        } else {
            log::warn!("transcribe: skipping unreadable audio track {path}");
        }
    }
    if tracks.is_empty() {
        return Err("no decodable audio in this recording".into());
    }
    if mix_audio && tracks.len() >= 2 {
        Ok(mix_tracks(&tracks[0], &tracks[1]))
    } else {
        Ok(tracks.remove(0))
    }
}

/// Load (or reuse the cached) engine for `model_id` and transcribe.
fn run_inference(app: &AppHandle, model_id: &str, samples: &[f32]) -> Result<String, String> {
    let path = ensure_model(app, model_id)?;
    let mut guard = engine_cache()
        .lock()
        .map_err(|_| "transcription engine lock poisoned")?;
    let needs_load = guard
        .as_ref()
        .map(|cached| cached.model_id != model_id || cached.path != path)
        .unwrap_or(true);
    if needs_load {
        log::info!("transcribe: loading whisper model {model_id}");
        let engine =
            WhisperEngine::load(&path).map_err(|e| format!("could not load model: {e}"))?;
        *guard = Some(CachedEngine {
            model_id: model_id.to_string(),
            path,
            engine,
        });
    }
    let cached = guard.as_mut().expect("engine just loaded");
    let multilingual = cached.engine.capabilities().languages.len() > 1;
    let options = WhisperInferenceParams {
        // English-only models ignore the hint; multilingual models auto-detect
        // so non-English meetings work out of the box.
        language: (!multilingual).then(|| "en".to_string()),
        ..Default::default()
    };
    let result = cached
        .engine
        .transcribe_with(samples, &options)
        .map_err(|e| format!("transcription failed: {e}"))?;
    Ok(result.text)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionStarted {
    pub conversation_id: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionFinished {
    pub conversation_id: i64,
    pub memory_id: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionFailed {
    pub conversation_id: i64,
    pub error: String,
}

/// Kick off background transcription for a conversation. Emits
/// `transcription-started`, then `transcription-finished` (after the memory is
/// persisted) or `transcription-failed`. Safe to call twice; duplicates are
/// dropped.
pub fn spawn_for(app: AppHandle, state: std::sync::Arc<MonitorState>, conversation_id: i64) {
    {
        let mut guard = match in_flight().lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if guard.contains(&conversation_id) {
            return;
        }
        guard.push(conversation_id);
    }
    std::thread::spawn(move || {
        let _ = app.emit(
            "transcription-started",
            &TranscriptionStarted { conversation_id },
        );
        let result = transcribe_and_persist(&app, &state, conversation_id);
        if let Ok(mut guard) = in_flight().lock() {
            guard.retain(|id| *id != conversation_id);
        }
        match result {
            Ok(memory) => {
                let _ = app.emit(
                    "transcription-finished",
                    &TranscriptionFinished {
                        conversation_id,
                        memory_id: memory.id,
                    },
                );
            }
            Err(error) => {
                log::error!("transcribe: conversation {conversation_id} failed: {error}");
                let _ = app.emit(
                    "transcription-failed",
                    &TranscriptionFailed {
                        conversation_id,
                        error,
                    },
                );
            }
        }
    });
}

/// Full pipeline for one conversation: load audio, transcribe, persist.
fn transcribe_and_persist(
    app: &AppHandle,
    state: &std::sync::Arc<MonitorState>,
    conversation_id: i64,
) -> Result<db::MemoryRow, String> {
    let db = state.db();
    let conversation = db
        .get_conversation(conversation_id)?
        .ok_or_else(|| format!("conversation {conversation_id} was not found"))?;
    if conversation
        .transcript
        .as_deref()
        .map(|t| !t.trim().is_empty())
        .unwrap_or(false)
    {
        return Err("this recording already has a transcript".into());
    }
    let paths: Vec<String> = conversation
        .audio_path
        .map(|raw| {
            raw.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if paths.is_empty() {
        return Err("this recording has no captured audio".into());
    }
    let mix_audio = db
        .get_setting("mixAudio")
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(true);
    let model_id = canonical_model_id(
        &db.get_setting("transcriptionModel")
            .ok()
            .flatten()
            .unwrap_or_default(),
    );

    // Release the shared DB lock before decoding/resampling the WAVs and the
    // long inference pass — both can take many seconds.
    drop(db);
    let samples = prepare_samples(&paths, mix_audio)?;
    let text = run_inference(app, &model_id, &samples)?;
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("no speech was detected in this recording".into());
    }

    let db = state.db();
    super::persist_transcript(&db, app, conversation_id, &text)
}


#[cfg(test)]
mod hardening_tests {
    use super::*;

    #[test]
    fn model_file_names_cannot_escape_the_models_dir() {
        assert_eq!(ggml_file_name("../../memento.db"), "memento.db");
        assert_eq!(ggml_file_name("/etc/passwd"), "passwd");
        assert_eq!(ggml_file_name(".."), "invalid-model");
        assert_eq!(ggml_file_name(MODEL_TINY_EN), "ggml-tiny.en.bin");
    }

    #[test]
    fn loads_legacy_float_wavs() {
        let path = std::env::temp_dir().join(format!("memento-tr-float-{}.wav", std::process::id()));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for s in [0.0f32, 0.5, -0.5] {
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
        let (samples, rate) = load_wav_mono(&path).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(samples, vec![0.0, 0.5, -0.5]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn downsampling_never_panics_and_has_expected_length() {
        for len in [1usize, 2, 3, 47, 48, 49, 4_801] {
            let input = vec![0.5f32; len];
            let out = resample_to_16k(&input, 48_000);
            assert_eq!(out.len(), ((len as f64) / 3.0).round() as usize);
            assert!(out.iter().all(|s| (*s - 0.5).abs() < 1e-6));
        }
        assert_eq!(resample_to_16k(&[0.1, 0.2], 8_000).len(), 4);
    }
}
