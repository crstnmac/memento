//! Meeting recording. Microphone capture uses cpal and system audio uses
//! Apple's ScreenCaptureKit on macOS 13 and later.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::state::MonitorState;

const SYSTEM_RATE: u32 = 48_000;
/// Safety valve only: audio is drained to disk every `DRAIN_INTERVAL`, so the
/// in-memory buffer normally holds a fraction of a second. If the disk stalls
/// this bounds memory (~10 minutes of mono 48k).
const MAX_BUFFERED_SAMPLES: usize = 10 * 60 * 48_000;
const DRAIN_INTERVAL: Duration = Duration::from_millis(200);
/// How often the WAV header is checkpointed, so a crash or force-quit leaves a
/// playable file instead of losing the whole meeting.
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(5);
/// A microphone that delivers no frames for this long is treated as dead.
const NO_FRAMES_TIMEOUT: Duration = Duration::from_secs(15);
/// Reopen attempts after a device error before the mic track is abandoned.
const MAX_REOPEN_ATTEMPTS: u32 = 20;
const MAX_REOPEN_DELAY: Duration = Duration::from_secs(10);

/// Sends user-visible warnings for one recording to the UI.
type Warn = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RecordingWarning {
    conversation_id: i64,
    message: String,
}

fn make_warner(app: &AppHandle, conversation_id: i64) -> Warn {
    let app = app.clone();
    Arc::new(move |message: &str| {
        log::warn!("recording #{conversation_id}: {message}");
        let _ = app.emit(
            "recording-warning",
            &RecordingWarning {
                conversation_id,
                message: message.to_string(),
            },
        );
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordingMode {
    Mic,
    System,
    Both,
}

impl RecordingMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "mic" => Some(Self::Mic),
            "system" => Some(Self::System),
            "both" => Some(Self::Both),
            _ => None,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingResult {
    pub conversation_id: i64,
    pub mic_path: Option<String>,
    pub system_path: Option<String>,
    pub duration_ms: i64,
    pub kind: String,
}

pub struct ActiveRecording {
    pub conversation_id: i64,
    pub started_ms: i64,
    mic_stop: Arc<AtomicBool>,
    system_stop: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
    /// Samples each capture thread has persisted to its WAV so far.
    mic_written: Arc<AtomicU64>,
    system_written: Arc<AtomicU64>,
    mic_path: PathBuf,
    system_path: PathBuf,
    kind: String,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn title_for_recording(kind: &str) -> String {
    let label = if kind == "voice-note" {
        "Voice note"
    } else {
        "Meeting recording"
    };
    // Local wall-clock time (the previous formatter showed UTC).
    format!("{label} · {}", chrono::Local::now().format("%H:%M"))
}

/// Audio handed over by a capture callback, plus when its first sample arrived.
#[derive(Default)]
struct Capture {
    samples: Vec<f32>,
    first_at: Option<Instant>,
}

impl Capture {
    /// Append mono samples captured at `rate`. The first batch after a drain is
    /// stamped with the (estimated) time its first sample was captured, which
    /// the sink uses to place the track on the recording's timeline.
    fn push(&mut self, mono: &[f32], rate: u32) {
        if self.first_at.is_none() {
            let length = Duration::from_secs_f64(mono.len() as f64 / f64::from(rate.max(1)));
            let now = Instant::now();
            self.first_at = Some(now.checked_sub(length).unwrap_or(now));
        }
        self.samples.extend_from_slice(mono);
        trim_to_cap(&mut self.samples);
    }
}

/// Number of silent samples to write so a track whose first sample arrived
/// `offset` after the recording started lines up with t=0 (`already_written`
/// samples are in the file at `rate`).
fn silence_samples(offset: Duration, already_written: u64, rate: u32) -> u64 {
    let target = (offset.as_secs_f64() * f64::from(rate)).round() as u64;
    target.saturating_sub(already_written)
}

/// Streaming linear-interpolation resampler, used only when the input device's
/// rate changes mid-recording so the WAV keeps a single sample rate.
struct LinearResampler {
    step: f64,
    pos: f64,
    carry: Vec<f32>,
}

impl LinearResampler {
    fn new(from: u32, to: u32) -> Self {
        Self {
            step: f64::from(from) / f64::from(to.max(1)),
            pos: 0.0,
            carry: Vec::new(),
        }
    }

    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.carry.extend_from_slice(input);
        let mut out = Vec::new();
        while self.pos + 1.0 < self.carry.len() as f64 {
            let i = self.pos as usize;
            let t = (self.pos - i as f64) as f32;
            out.push(self.carry[i] * (1.0 - t) + self.carry[i + 1] * t);
            self.pos += self.step;
        }
        let consumed = (self.pos as usize).min(self.carry.len());
        self.carry.drain(..consumed);
        self.pos -= consumed as f64;
        out
    }
}

fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16
}

/// Incrementally writes a capture thread's samples to a 16-bit mono WAV. The
/// audio callback only appends to `buffer`; this drains it to disk on the
/// capture thread, so RAM use is O(drain interval) rather than O(meeting
/// length) and nothing is lost if the app dies mid-recording.
///
/// Timeline: every sink knows the recording's `origin`. The first samples of
/// each segment (the start of the recording, and after a device change) are
/// preceded by silence for the time elapsed since the file's end, so mic and
/// system tracks both begin at t=0 and `mix_tracks` sees them aligned. If the
/// input device's rate changes, samples are resampled to the file's rate.
struct WavSink {
    writer: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>,
    buffer: Arc<Mutex<Capture>>,
    /// Real (non-padding) samples persisted; used to tell if a track has audio.
    written: Arc<AtomicU64>,
    last_checkpoint: Instant,
    origin: Instant,
    rate: u32,
    /// Samples in the file, including alignment silence.
    total: u64,
    segment_pending: bool,
    resampler: Option<LinearResampler>,
}

impl WavSink {
    fn create(
        path: &Path,
        rate: u32,
        buffer: Arc<Mutex<Capture>>,
        written: Arc<AtomicU64>,
        origin: Instant,
    ) -> Result<Self, String> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let writer = hound::WavWriter::create(path, spec)
            .map_err(|e| format!("could not create {}: {e}", path.display()))?;
        Ok(Self {
            writer: Some(writer),
            buffer,
            written,
            last_checkpoint: Instant::now(),
            origin,
            rate,
            total: 0,
            segment_pending: true,
            resampler: None,
        })
    }

    /// The next audio is a new segment (the input device was replaced): pad any
    /// gap with silence so later audio stays aligned with the recording clock.
    fn begin_segment(&mut self) {
        self.segment_pending = true;
    }

    /// Declare the rate of audio the capture callbacks produce from now on.
    /// Call only after the old stream is dropped and drained.
    fn set_source_rate(&mut self, rate: u32) {
        self.resampler = (rate != self.rate).then(|| LinearResampler::new(rate, self.rate));
    }

    fn write_samples(&mut self, samples: impl Iterator<Item = f32>) -> bool {
        let Some(writer) = self.writer.as_mut() else {
            return false;
        };
        for sample in samples {
            if writer.write_sample(to_i16(sample)).is_err() {
                // Disk full / removed: stop writing, keep what is already saved.
                log::error!("recording: write failed; further audio on this track is dropped");
                self.writer = None;
                return false;
            }
            self.total += 1;
        }
        true
    }

    /// Persist everything captured so far. Returns how many source samples
    /// arrived (0 means the device delivered nothing since the last drain).
    fn drain(&mut self) -> usize {
        let chunk = std::mem::take(
            &mut *self
                .buffer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        let arrived = chunk.samples.len();
        if arrived == 0 || self.writer.is_none() {
            return arrived;
        }
        if self.segment_pending {
            self.segment_pending = false;
            if let Some(first_at) = chunk.first_at {
                let offset = first_at.saturating_duration_since(self.origin);
                let pad = silence_samples(offset, self.total, self.rate);
                if !self.write_samples(std::iter::repeat_n(0.0f32, pad as usize)) {
                    return arrived;
                }
            }
        }
        let converted;
        let samples: &[f32] = match self.resampler.as_mut() {
            Some(resampler) => {
                converted = resampler.process(&chunk.samples);
                &converted
            }
            None => &chunk.samples,
        };
        if !self.write_samples(samples.iter().copied()) {
            return arrived;
        }
        self.written.fetch_add(arrived as u64, Ordering::Relaxed);
        if self.last_checkpoint.elapsed() >= CHECKPOINT_INTERVAL {
            self.last_checkpoint = Instant::now();
            if let Some(writer) = self.writer.as_mut() {
                let _ = writer.flush();
            }
        }
        arrived
    }

    fn finish(mut self) {
        self.drain();
        if let Some(writer) = self.writer.take() {
            if let Err(error) = writer.finalize() {
                log::error!("recording: could not finalize WAV: {error}");
            }
        }
    }
}

/// Start a recording in the given mode. Creates a conversation row up front.
///
/// With `fallback_to_mic`, a "both"-mode start degrades to whichever track
/// opens successfully instead of failing outright — used by micwatch so an
/// ungranted Screen Recording permission can't silently kill auto-start.
///
/// The shared DB lock is held only for the conversation-row bookkeeping, never
/// while opening audio devices: opening system audio can block on a macOS
/// Screen Recording permission prompt, which must not freeze the whole app.
pub fn start(
    app: &AppHandle,
    state: &MonitorState,
    mode: RecordingMode,
    kind: &str,
    fallback_to_mic: bool,
) -> Result<ActiveRecording, String> {
    let started_ms = now_ms();
    let origin = Instant::now();
    let kind = if kind == "voice-note" {
        "voice-note"
    } else {
        "meeting"
    };
    let title = title_for_recording(kind);
    let dir = crate::db::recordings_dir()?;
    let conversation_id = state
        .db()
        .insert_conversation(&title, started_ms, None, kind)?;
    let mic_path = dir.join(format!("mic-{conversation_id}.wav"));
    let system_path = dir.join(format!("system-{conversation_id}.wav"));

    let mic_stop = Arc::new(AtomicBool::new(false));
    let system_stop = Arc::new(AtomicBool::new(false));
    let mic_written = Arc::new(AtomicU64::new(0));
    let system_written = Arc::new(AtomicU64::new(0));
    let mut handles: Vec<JoinHandle<()>> = Vec::new();
    let warn = make_warner(app, conversation_id);

    let mut mic_ok = false;
    let mut mic_error: Option<String> = None;
    if matches!(mode, RecordingMode::Mic | RecordingMode::Both) {
        match spawn_mic(&mic_stop, &mic_path, &mic_written, origin, warn.clone()) {
            Ok(handle) => {
                handles.push(handle);
                mic_ok = true;
            }
            Err(error) => mic_error = Some(error),
        }
    }
    let mut system_ok = false;
    let mut system_error: Option<String> = None;
    if matches!(mode, RecordingMode::System | RecordingMode::Both) {
        match spawn_system(&system_stop, &system_path, &system_written, origin) {
            Ok(handle) => {
                handles.push(handle);
                system_ok = true;
            }
            Err(error) => system_error = Some(error),
        }
    }

    let abort = |mic_stop: &Arc<AtomicBool>,
                 system_stop: &Arc<AtomicBool>,
                 handles: &mut Vec<JoinHandle<()>>| {
        stop_capture_handles(mic_stop, system_stop, handles);
        let _ = std::fs::remove_file(&mic_path);
        let _ = std::fs::remove_file(&system_path);
        let _ = state.db().delete_conversation(conversation_id);
    };

    let degraded = fallback_to_mic && mode == RecordingMode::Both;
    if !mic_ok && !system_ok {
        abort(&mic_stop, &system_stop, &mut handles);
        return Err(mic_error
            .or(system_error)
            .unwrap_or_else(|| "no audio source could be opened".into()));
    }
    if degraded {
        if let Some(error) = &mic_error {
            log::warn!("recording: microphone unavailable, continuing with system audio only: {error}");
        }
        if let Some(error) = &system_error {
            log::warn!("recording: system audio unavailable, continuing with the microphone only: {error}");
        }
    } else if let Some(error) = mic_error.or(system_error) {
        abort(&mic_stop, &system_stop, &mut handles);
        return Err(error);
    }

    Ok(ActiveRecording {
        conversation_id,
        started_ms,
        mic_stop,
        system_stop,
        handles,
        mic_written,
        system_written,
        mic_path,
        system_path,
        kind: kind.to_string(),
    })
}

/// Stop a recording and finalize its WAV files. The capture threads have been
/// streaming to disk all along, so this only joins them (each flushes its last
/// buffer and finalizes) and records the result — no big in-memory encode.
pub fn stop(state: &MonitorState, recording: ActiveRecording) -> Result<RecordingResult, String> {
    recording.mic_stop.store(true, Ordering::Relaxed);
    recording.system_stop.store(true, Ordering::Relaxed);
    for handle in recording.handles {
        let _ = handle.join();
    }

    let id = recording.conversation_id;
    let mic_has_audio = recording.mic_written.load(Ordering::Relaxed) > 0;
    let system_has_audio = recording.system_written.load(Ordering::Relaxed) > 0;
    // An opened-but-silent track leaves an empty WAV behind; remove it.
    if !mic_has_audio {
        let _ = std::fs::remove_file(&recording.mic_path);
    }
    if !system_has_audio {
        let _ = std::fs::remove_file(&recording.system_path);
    }
    if !mic_has_audio && !system_has_audio {
        let _ = state.db().delete_conversation(id);
        return Err("recording stopped without receiving any audio frames; check microphone and Screen Recording permissions".into());
    }

    let mic_path = mic_has_audio.then(|| recording.mic_path.to_string_lossy().to_string());
    let system_path =
        system_has_audio.then(|| recording.system_path.to_string_lossy().to_string());
    let ended_ms = now_ms();
    let duration_ms = (ended_ms - recording.started_ms).max(0);

    let joined = [mic_path.as_deref(), system_path.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(",");
    if let Err(error) = state.db().finish_recording(
        id,
        (!joined.is_empty()).then_some(joined.as_str()),
        ended_ms,
        duration_ms,
    ) {
        // Do not leave files detached from the database if persistence fails.
        for path in [&recording.mic_path, &recording.system_path] {
            let _ = std::fs::remove_file(path);
        }
        return Err(error);
    }

    Ok(RecordingResult {
        conversation_id: id,
        mic_path,
        system_path,
        duration_ms,
        kind: recording.kind,
    })
}

/// Signal every capture thread to stop and join it before unwinding a failed
/// start, so no stream outlives the recording it belonged to.
fn stop_capture_handles(
    mic_stop: &Arc<AtomicBool>,
    system_stop: &Arc<AtomicBool>,
    handles: &mut Vec<JoinHandle<()>>,
) {
    mic_stop.store(true, Ordering::Relaxed);
    system_stop.store(true, Ordering::Relaxed);
    for handle in handles.drain(..) {
        let _ = handle.join();
    }
}

struct MicInput {
    // Held only to keep capturing; dropping it stops the callbacks.
    _stream: cpal::Stream,
    rate: u32,
    name: String,
}

/// Open the current default input device. Callbacks push mono samples into
/// `capture`; any stream error sets `failed` so the capture thread can rebuild.
fn open_mic(capture: &Arc<Mutex<Capture>>, failed: &Arc<AtomicBool>) -> Result<MicInput, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or("microphone: no input device is available")?;
    let name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "the default microphone".to_string());
    let config = device
        .default_input_config()
        .map_err(|e| format!("microphone: no supported input configuration: {e}"))?;
    let rate = config.sample_rate();
    let channels = config.channels().max(1) as usize;
    failed.store(false, Ordering::Relaxed);
    let on_error = |failed: &Arc<AtomicBool>| {
        let failed = Arc::clone(failed);
        move |err: cpal::Error| {
            log::warn!("microphone stream error: {err}");
            failed.store(true, Ordering::Relaxed);
        }
    };

    macro_rules! build {
        ($ty:ty) => {{
            let capture = Arc::clone(capture);
            device.build_input_stream(
                config.config(),
                move |data: &[$ty], _| push_mono(data, channels, rate, &capture),
                on_error(failed),
                None,
            )
        }};
    }
    let stream_result = match config.sample_format() {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::U16 => build!(u16),
        cpal::SampleFormat::I32 => build!(i32),
        cpal::SampleFormat::F64 => build!(f64),
        other => return Err(format!("microphone: unsupported sample format {other:?}")),
    };
    let stream = stream_result.map_err(|e| format!("microphone: could not build stream: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("microphone: could not start stream: {e}"))?;
    Ok(MicInput {
        _stream: stream,
        rate,
        name,
    })
}

/// Delay before reopen attempt number `attempt` (1-based): 1s, 2s, 4s, 8s,
/// then capped.
fn reopen_delay(attempt: u32) -> Duration {
    let secs = 1u64 << attempt.saturating_sub(1).min(4);
    Duration::from_secs(secs).min(MAX_REOPEN_DELAY)
}

/// Microphone capture thread. If the stream errors (device unplugged, AirPods
/// disconnected, route change) or stops delivering frames, it rebuilds the
/// stream on the current default input and keeps appending to the same WAV;
/// see `WavSink` for how rate changes and the time gap are handled.
fn spawn_mic(
    stop: &Arc<AtomicBool>,
    path: &Path,
    written: &Arc<AtomicU64>,
    origin: Instant,
    warn: Warn,
) -> Result<JoinHandle<()>, String> {
    let capture: Arc<Mutex<Capture>> = Arc::new(Mutex::new(Capture::default()));
    let failed = Arc::new(AtomicBool::new(false));
    let first = open_mic(&capture, &failed)?;
    let mut sink = match WavSink::create(path, first.rate, Arc::clone(&capture), Arc::clone(written), origin) {
        Ok(sink) => sink,
        Err(error) => {
            let _ = std::fs::remove_file(path);
            return Err(error);
        }
    };
    let stop_flag = Arc::clone(stop);

    Ok(std::thread::spawn(move || {
        let mut input = Some(first);
        let mut attempts: u32 = 0;
        let mut next_try = Instant::now();
        let mut gave_up = false;
        let mut last_frames = Instant::now();
        let mut stall_warned = false;
        while !stop_flag.load(Ordering::Relaxed) {
            std::thread::sleep(DRAIN_INTERVAL);
            if sink.drain() > 0 {
                last_frames = Instant::now();
                stall_warned = false;
            }
            if input.is_some() {
                let errored = failed.swap(false, Ordering::Relaxed);
                let stalled = last_frames.elapsed() >= NO_FRAMES_TIMEOUT;
                if stalled && !stall_warned {
                    stall_warned = true;
                    warn("No sound is reaching Memento from your microphone. It may be muted or disconnected.");
                }
                if errored || stalled {
                    // Drop the old stream first so the final drain sees every
                    // sample at the old rate.
                    input = None;
                    sink.drain();
                    sink.begin_segment();
                    attempts = 0;
                    gave_up = false;
                    next_try = Instant::now();
                    if errored {
                        warn("Your microphone was disconnected. Memento is looking for another one.");
                    }
                }
            }
            if input.is_none() && !gave_up && Instant::now() >= next_try {
                match open_mic(&capture, &failed) {
                    Ok(new_input) => {
                        sink.set_source_rate(new_input.rate);
                        warn(&format!(
                            "Your microphone changed — Memento switched to {}.",
                            new_input.name
                        ));
                        input = Some(new_input);
                        last_frames = Instant::now();
                    }
                    Err(error) => {
                        attempts += 1;
                        log::warn!("recording: reopening microphone failed (attempt {attempts}): {error}");
                        if attempts == 1 {
                            warn("Memento couldn't find a microphone and is still trying.");
                        }
                        if attempts >= MAX_REOPEN_ATTEMPTS {
                            gave_up = true;
                            warn("Memento couldn't reconnect to a microphone. The rest of this recording won't include your voice.");
                        } else {
                            next_try = Instant::now() + reopen_delay(attempts);
                        }
                    }
                }
            }
        }
        // Stop the callbacks first so the final drain sees every sample.
        drop(input);
        sink.finish();
    }))
}

/// Downmix an interleaved frame to mono f32 and append.
trait AsF32 {
    fn to_f32(self) -> f32;
}
impl AsF32 for f32 {
    fn to_f32(self) -> f32 {
        self
    }
}
impl AsF32 for i16 {
    fn to_f32(self) -> f32 {
        self as f32 / 32768.0
    }
}
impl AsF32 for u16 {
    fn to_f32(self) -> f32 {
        (self as f32 - 32768.0) / 32768.0
    }
}
impl AsF32 for i32 {
    fn to_f32(self) -> f32 {
        self as f32 / 2147483648.0
    }
}
impl AsF32 for f64 {
    fn to_f32(self) -> f32 {
        self as f32
    }
}

fn push_mono<T: AsF32 + std::marker::Copy>(
    data: &[T],
    channels: usize,
    rate: u32,
    samples: &Arc<Mutex<Capture>>,
) {
    if data.is_empty() {
        return;
    }
    let mut mono = Vec::with_capacity(data.len() / channels + 1);
    for frame in data.chunks(channels) {
        let sum: f32 = frame.iter().map(|s| s.to_f32()).sum();
        mono.push(sum / channels as f32);
    }
    // Never panic on the realtime audio thread because of a poisoned lock.
    let mut guard = samples.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.push(&mono, rate);
}

/// Memory safety valve: if the writer falls behind (stalled disk), drop the
/// oldest buffered audio rather than growing without bound.
fn trim_to_cap(samples: &mut Vec<f32>) {
    if samples.len() > MAX_BUFFERED_SAMPLES {
        let excess = samples.len() - MAX_BUFFERED_SAMPLES;
        samples.drain(0..excess);
    }
}

/// Capture system output with ScreenCaptureKit. Initialisation occurs on the
/// owning capture thread, while this handshake ensures callers see permission
/// and stream setup failures before reporting that recording has started.
#[cfg(target_os = "macos")]
fn spawn_system(
    stop: &Arc<AtomicBool>,
    path: &Path,
    written: &Arc<AtomicU64>,
    origin: Instant,
) -> Result<JoinHandle<()>, String> {
    use screencapturekit::prelude::*;

    let stop_flag = Arc::clone(stop);
    let samples: Arc<Mutex<Capture>> = Arc::new(Mutex::new(Capture::default()));
    let mut sink = WavSink::create(path, SYSTEM_RATE, Arc::clone(&samples), Arc::clone(written), origin)?;
    let cleanup_path = path.to_path_buf();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
    let handle = std::thread::spawn(move || {
        let content = match SCShareableContent::get() {
            Ok(content) => content,
            Err(error) => {
                let detail = error.to_string();
                let guidance = if detail.to_lowercase().contains("declined")
                    || detail.to_lowercase().contains("denied")
                {
                    " Allow Memento in System Settings → Privacy & Security → Screen & System Audio Recording, then quit and reopen it."
                } else {
                    ""
                };
                let _ = ready_tx.send(Err(format!(
                    "system audio requires Screen Recording permission: {error}.{guidance}"
                )));
                return;
            }
        };
        let Some(display) = content.displays().into_iter().next() else {
            let _ = ready_tx.send(Err("system audio: no display is available".into()));
            return;
        };
        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build();
        let config = SCStreamConfiguration::new()
            .with_captures_audio(true)
            .with_sample_rate(SYSTEM_RATE as i32)
            .with_channel_count(1);
        let mut stream = SCStream::new(&filter, &config);
        let output_samples = Arc::clone(&samples);
        if stream
            .add_output_handler(
                move |sample: CMSampleBuffer, output_type: SCStreamOutputType| {
                    if output_type != SCStreamOutputType::Audio {
                        return;
                    }
                    let Some(buffers) = sample.audio_buffer_list() else {
                        return;
                    };
                    let Some(format) = sample.format_description() else { return };
                    let bits = format.audio_bits_per_channel().unwrap_or(0);
                    let is_float = format.audio_is_float();
                    let big_endian = format.audio_is_big_endian();
                    let supported_layout = bits == 32 || (!is_float && bits == 16);
                    if big_endian || !supported_layout {
                        log::warn!("system audio: unsupported PCM layout (float={is_float}, bits={bits}, big_endian={big_endian})");
                        return;
                    }
                    let mut captured = Vec::new();
                    for buffer in &buffers {
                        let channels = buffer.number_channels.max(1) as usize;
                        let decoded: Vec<f32> = match (is_float, bits) {
                            (true, 32) => buffer.data().chunks_exact(4)
                                .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap())).collect(),
                            (false, 16) => buffer.data().chunks_exact(2)
                                .map(|bytes| i16::from_le_bytes(bytes.try_into().unwrap()) as f32 / 32768.0).collect(),
                            (false, 32) => buffer.data().chunks_exact(4)
                                .map(|bytes| i32::from_le_bytes(bytes.try_into().unwrap()) as f32 / 2147483648.0).collect(),
                            _ => Vec::new(),
                        };
                        for frame in decoded.chunks(channels) {
                            captured.push(frame.iter().sum::<f32>() / frame.len() as f32);
                        }
                    }
                    if captured.is_empty() {
                        return;
                    }
                    if let Ok(mut guard) = output_samples.lock() {
                        guard.push(&captured, SYSTEM_RATE);
                    }
                },
                SCStreamOutputType::Audio,
            )
            .is_none()
        {
            let _ = ready_tx.send(Err("system audio: could not install output handler".into()));
            return;
        }
        if let Err(error) = stream.start_capture() {
            let _ = ready_tx.send(Err(format!(
                "system audio: could not start capture: {error}"
            )));
            return;
        }
        let _ = ready_tx.send(Ok(()));
        while !stop_flag.load(Ordering::Relaxed) {
            std::thread::sleep(DRAIN_INTERVAL);
            sink.drain();
        }
        if let Err(error) = stream.stop_capture() {
            log::warn!("system audio: could not stop capture cleanly: {error}");
        }
        sink.finish();
    });

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(error)) => {
            let _ = handle.join();
            let _ = std::fs::remove_file(&cleanup_path);
            Err(error)
        }
        Err(_) => {
            let _ = handle.join();
            let _ = std::fs::remove_file(&cleanup_path);
            Err("system audio capture stopped during startup".into())
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn spawn_system(
    _stop: &Arc<AtomicBool>,
    _path: &Path,
    _written: &Arc<AtomicU64>,
    _origin: Instant,
) -> Result<JoinHandle<()>, String> {
    Err("system audio capture is only available on macOS".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_wav(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("memento-audio-{name}-{}.wav", std::process::id()))
    }

    fn new_sink(path: &Path, rate: u32, origin: Instant) -> (WavSink, Arc<Mutex<Capture>>, Arc<AtomicU64>) {
        let capture = Arc::new(Mutex::new(Capture::default()));
        let written = Arc::new(AtomicU64::new(0));
        let sink = WavSink::create(path, rate, Arc::clone(&capture), Arc::clone(&written), origin)
            .expect("create");
        (sink, capture, written)
    }

    fn feed(capture: &Arc<Mutex<Capture>>, samples: &[f32], first_at: Instant) {
        let mut guard = capture.lock().unwrap();
        guard.samples.extend_from_slice(samples);
        guard.first_at.get_or_insert(first_at);
    }

    #[test]
    fn sink_streams_samples_to_a_valid_16_bit_wav() {
        let path = temp_wav("stream");
        let origin = Instant::now();
        let (mut sink, capture, written) = new_sink(&path, 48_000, origin);
        for _ in 0..3 {
            feed(&capture, &[0.25f32; 1_000], origin);
            sink.drain();
        }
        feed(&capture, &[0.25f32; 500], origin);
        sink.finish();
        assert_eq!(written.load(Ordering::Relaxed), 3_500);
        let reader = hound::WavReader::open(&path).expect("valid wav");
        assert_eq!(reader.spec().sample_rate, 48_000);
        assert_eq!(reader.spec().bits_per_sample, 16);
        assert_eq!(reader.spec().sample_format, hound::SampleFormat::Int);
        assert_eq!(reader.len(), 3_500);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn quantization_round_trips_within_tolerance() {
        let path = temp_wav("quant");
        let origin = Instant::now();
        let (sink, capture, _) = new_sink(&path, 16_000, origin);
        let input: Vec<f32> = (0..4_000)
            .map(|i| (i as f32 * 0.05).sin() * 0.9)
            .chain([1.0, -1.0, 0.0, 1.5, -1.5])
            .collect();
        feed(&capture, &input, origin);
        sink.finish();
        let (decoded, rate) = crate::transcribe::load_wav_mono(&path).expect("load");
        assert_eq!(rate, 16_000);
        assert_eq!(decoded.len(), input.len());
        for (a, b) in input.iter().zip(&decoded) {
            assert!((a.clamp(-1.0, 1.0) - b).abs() < 1.0 / 16_000.0, "{a} vs {b}");
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn checkpointed_wav_is_readable_before_finalize() {
        let path = temp_wav("checkpoint");
        let origin = Instant::now();
        let (mut sink, capture, _) = new_sink(&path, 16_000, origin);
        feed(&capture, &[0.1f32; 2_000], origin);
        // Force a checkpoint, then simulate a crash by never finalizing.
        sink.last_checkpoint = Instant::now() - CHECKPOINT_INTERVAL;
        sink.drain();
        let reader = hound::WavReader::open(&path).expect("recoverable after crash");
        assert_eq!(reader.len(), 2_000);
        std::mem::forget(sink);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn buffer_is_bounded_when_the_writer_stalls() {
        let mut samples = vec![0.0f32; MAX_BUFFERED_SAMPLES + 123];
        trim_to_cap(&mut samples);
        assert_eq!(samples.len(), MAX_BUFFERED_SAMPLES);
    }

    #[test]
    fn silence_arithmetic() {
        let rate = 48_000;
        assert_eq!(silence_samples(Duration::from_millis(1_500), 0, rate), 72_000);
        assert_eq!(silence_samples(Duration::ZERO, 0, rate), 0);
        // Reconnect: file already holds 10s, audio resumes at 12.5s.
        assert_eq!(silence_samples(Duration::from_millis(12_500), 480_000, rate), 120_000);
        // Never negative when the file is already past the target.
        assert_eq!(silence_samples(Duration::from_secs(1), 96_000, rate), 0);
    }

    #[test]
    fn late_starting_track_is_padded_with_leading_silence() {
        let path = temp_wav("align");
        let origin = Instant::now();
        let (sink, capture, written) = new_sink(&path, 48_000, origin);
        feed(&capture, &[0.5f32; 480], origin + Duration::from_millis(250));
        sink.finish();
        // Padding is not counted as captured audio.
        assert_eq!(written.load(Ordering::Relaxed), 480);
        let mut reader = hound::WavReader::open(&path).unwrap();
        let samples: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(samples.len(), 12_000 + 480);
        assert!(samples[..12_000].iter().all(|s| *s == 0));
        assert!(samples[12_000..].iter().all(|s| *s != 0));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn gap_after_device_change_is_filled_and_rate_converted() {
        let path = temp_wav("segment");
        let origin = Instant::now();
        let (mut sink, capture, _) = new_sink(&path, 16_000, origin);
        feed(&capture, &[0.5f32; 16_000], origin); // 1s at 16 kHz
        sink.drain();
        // New device at 48 kHz resumes at t=2s: 1s gap, then 1s of audio.
        sink.begin_segment();
        sink.set_source_rate(48_000);
        feed(&capture, &[0.5f32; 48_000], origin + Duration::from_secs(2));
        sink.finish();
        let reader = hound::WavReader::open(&path).unwrap();
        let len = reader.len() as i64;
        assert!((len - 48_000).abs() < 8, "unexpected length {len}");
    }

    #[test]
    fn resampler_preserves_dc_and_scales_length() {
        let mut r = LinearResampler::new(44_100, 16_000);
        let mut out = Vec::new();
        for _ in 0..10 {
            out.extend(r.process(&[0.3f32; 4_410]));
        }
        assert!((out.len() as i64 - 16_000).abs() < 4);
        assert!(out.iter().all(|s| (s - 0.3).abs() < 1e-5));
    }

    #[test]
    fn reopen_backoff_is_bounded() {
        assert_eq!(reopen_delay(1), Duration::from_secs(1));
        assert_eq!(reopen_delay(2), Duration::from_secs(2));
        assert_eq!(reopen_delay(4), Duration::from_secs(8));
        assert_eq!(reopen_delay(5), MAX_REOPEN_DELAY);
        assert_eq!(reopen_delay(500), MAX_REOPEN_DELAY);
    }
}
