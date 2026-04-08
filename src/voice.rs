/// Voice input — audio capture + transcription.
///
/// Recording: uses system `arecord` (Linux) or `sox` if available.
/// Transcription: tries in priority order:
///   1. Local `whisper` CLI (OpenAI whisper or whisper.cpp)
///   2. OpenAI-compatible /v1/audio/transcriptions API endpoint
///      (reads OPENAI_API_KEY or WHISPER_API_KEY from env)
///
/// Usage:
///   /voice          — show status + setup instructions
///   /voice enable   — enable voice mode
///   /voice disable  — disable voice mode
///   Ctrl+R          — while voice mode is on: start/stop recording
///
/// The transcribed text is inserted directly into the input buffer.

use anyhow::{anyhow, Result};
use std::path::PathBuf;
use tokio::process::Command;
use std::process::Stdio;

// ── Availability checks ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum RecorderBackend {
    Arecord,
    Sox,
    Ffmpeg,
}

pub fn find_recorder() -> Option<RecorderBackend> {
    if which("arecord")  { Some(RecorderBackend::Arecord) }
    else if which("sox") { Some(RecorderBackend::Sox) }
    else if which("ffmpeg") { Some(RecorderBackend::Ffmpeg) }
    else { None }
}

pub fn local_whisper_available() -> bool {
    which("whisper") || which("whisper-cpp") || which("whisper.cpp")
}

pub fn voice_api_key() -> Option<String> {
    std::env::var("OPENAI_API_KEY").ok()
        .or_else(|| std::env::var("WHISPER_API_KEY").ok())
}

pub fn transcription_available() -> bool {
    local_whisper_available() || voice_api_key().is_some()
}

fn which(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── Temp file path ────────────────────────────────────────────────────────────

/// Returns the path for the voice recording WAV file.
///
/// IMPORTANT: Always use `std::env::temp_dir()` here, never hardcode `/tmp`.
/// On many systems (custom TMPDIR, NixOS, Arch with TMPDIR on separate partition,
/// macOS which uses /var/folders/..., etc.) the real temp dir is NOT /tmp.
/// Using the wrong dir means the recorder writes the WAV somewhere whisper
/// never finds it, producing a "Failed to load audio" error.
pub fn temp_wav_path() -> PathBuf {
    std::env::temp_dir().join("rustyclaw-voice.wav")
}

// ── Recording ─────────────────────────────────────────────────────────────────

/// Spawn the recorder process. Returns the child process handle.
/// The caller is responsible for killing it when recording should stop.
pub async fn start_recording(backend: &RecorderBackend) -> Result<tokio::process::Child> {
    let out = temp_wav_path();
    // Clean up any previous recording
    let _ = tokio::fs::remove_file(&out).await;

    let child = match backend {
        RecorderBackend::Arecord => {
            Command::new("arecord")
                .args([
                    "-f", "S16_LE",   // 16-bit signed little-endian
                    "-r", "16000",    // 16kHz (Whisper optimal)
                    "-c", "1",        // mono
                    "-t", "wav",
                    &out.display().to_string(),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?
        }
        RecorderBackend::Sox => {
            Command::new("sox")
                .args([
                    "-d",             // default audio device
                    "-r", "16000",
                    "-c", "1",
                    "-b", "16",
                    &out.display().to_string(),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?
        }
        RecorderBackend::Ffmpeg => {
            Command::new("ffmpeg")
                .args([
                    "-f", "alsa",
                    "-i", "default",
                    "-ar", "16000",
                    "-ac", "1",
                    "-y",
                    &out.display().to_string(),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?
        }
    };
    Ok(child)
}

// ── Transcription ─────────────────────────────────────────────────────────────

/// Transcribe the recorded WAV file. Returns the transcribed text.
pub async fn transcribe(
    api_url: Option<&str>,
    api_key: Option<&str>,
) -> Result<String> {
    let wav = temp_wav_path();
    if !wav.exists() {
        return Err(anyhow!("No recording found at {}", wav.display()));
    }

    // Try local whisper first (no API key needed, works offline)
    if local_whisper_available() {
        return transcribe_local(&wav).await;
    }

    // Fall back to OpenAI-compatible API
    let key = api_key
        .map(|s| s.to_string())
        .or_else(voice_api_key)
        .ok_or_else(|| anyhow!(
            "No transcription available.\n\
             Set OPENAI_API_KEY or WHISPER_API_KEY env var,\n\
             or install whisper: pip install openai-whisper"
        ))?;

    let url = api_url.unwrap_or("https://api.openai.com/v1/audio/transcriptions");
    transcribe_api(&wav, url, &key).await
}

async fn transcribe_local(wav: &std::path::Path) -> Result<String> {
    // Try whisper CLI tools in order
    for binary in &["whisper", "whisper-cpp", "whisper.cpp"] {
        if which(binary) {
            // Use std::env::temp_dir() — never hardcode /tmp.
            // TMPDIR can be /mnt/Storage/tmp, /var/folders/..., or any custom path.
            // The --output_dir passed to whisper MUST match so we can find the .txt output.
            let tmp_dir = std::env::temp_dir();
            let out = Command::new(binary)
                .args([
                    &wav.display().to_string(),
                    "--model", "base",
                    "--output_format", "txt",
                    "--fp16", "False",
                    "--output_dir", tmp_dir.to_str().unwrap_or("/tmp"),
                ])
                .output()
                .await?;

            if out.status.success() {
                // whisper writes <filename>.txt in output_dir
                let txt_path = tmp_dir
                    .join(wav.file_stem().unwrap_or_default())
                    .with_extension("txt");
                if let Ok(text) = tokio::fs::read_to_string(&txt_path).await {
                    let _ = tokio::fs::remove_file(&txt_path).await;
                    return Ok(text.trim().to_string());
                }
                // Some versions print to stdout
                return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
            }
        }
    }
    Err(anyhow!("Local whisper transcription failed"))
}

async fn transcribe_api(wav: &std::path::Path, url: &str, api_key: &str) -> Result<String> {
    let wav_bytes = tokio::fs::read(wav).await?;
    let filename = wav.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.wav")
        .to_string();

    let part = reqwest::multipart::Part::bytes(wav_bytes)
        .file_name(filename)
        .mime_str("audio/wav")?;
    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", "whisper-1")
        .text("response_format", "text");

    let client = reqwest::Client::new();
    let resp = client.post(url)
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Transcription API error {}: {}", status, body));
    }

    Ok(resp.text().await?.trim().to_string())
}

// ── Text-to-Speech via piper ──────────────────────────────────────────────────

pub fn piper_available() -> bool {
    which("piper") || which("piper-tts")
}

pub fn audio_player_available() -> bool {
    which("aplay") || which("paplay") || which("mpv") || which("ffplay") || which("play")
}

/// Find the default piper voice model (.onnx) by searching known locations.
/// Prefers en_US-lessac-high (natural US female), falls back to any .onnx found.
pub fn find_default_voice() -> Option<String> {
    // Preferred voice file names (plain or under Arch system-package subpath)
    let preferred_subpaths = [
        "en_US-lessac-high.onnx",
        "en/en_US/lessac/high/en_US-lessac-high.onnx",
        "en_US-jenny-dioco-medium.onnx",
        "en/en_US/jenny_dioco/medium/en_US-jenny_dioco-medium.onnx",
        "en_GB-cori-high.onnx",
        "en/en_GB/cori/high/en_GB-cori-high.onnx",
    ];
    let search_roots: Vec<std::path::PathBuf> = [
        dirs::home_dir().map(|h| h.join(".local/share/piper")),
        Some(std::path::PathBuf::from("/usr/share/piper-voices")),
        Some(std::path::PathBuf::from("/usr/lib/piper")),
        Some(std::path::PathBuf::from("/usr/share/piper")),
    ]
    .into_iter()
    .flatten()
    .collect();

    // Check preferred relative paths under each root first
    for root in &search_roots {
        for subpath in &preferred_subpaths {
            let p = root.join(subpath);
            if p.exists() { return Some(p.display().to_string()); }
        }
    }
    // Fall back to any .onnx found by recursive walk (depth ≤ 5)
    for root in &search_roots {
        if let Some(found) = find_onnx_recursive(root, 5) {
            return Some(found);
        }
    }
    None
}

/// Find all installed piper voice models (.onnx files) across known locations.
/// Returns Vec of (display_name, full_path) sorted alphabetically by name.
pub fn find_all_voices() -> Vec<(String, String)> {
    let search_roots: Vec<std::path::PathBuf> = [
        dirs::home_dir().map(|h| h.join(".local/share/piper")),
        Some(std::path::PathBuf::from("/usr/share/piper-voices")),
        Some(std::path::PathBuf::from("/usr/lib/piper")),
        Some(std::path::PathBuf::from("/usr/share/piper")),
    ]
    .into_iter()
    .flatten()
    .filter(|p| p.exists())
    .collect();

    let mut voices = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for root in &search_roots {
        collect_onnx_recursive(root, 5, &mut voices, &mut seen);
    }
    voices.sort_by(|a, b| a.0.cmp(&b.0));
    voices
}

fn collect_onnx_recursive(
    dir: &std::path::Path,
    depth: u8,
    out: &mut Vec<(String, String)>,
    seen: &mut std::collections::HashSet<String>,
) {
    if depth == 0 { return; }
    let entries = match std::fs::read_dir(dir) { Ok(e) => e, Err(_) => return };
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_owned();
        if path.is_file() && name.ends_with(".onnx") && !name.ends_with(".onnx.json") {
            let full = path.display().to_string();
            if seen.insert(full.clone()) {
                // Display name: strip .onnx, replace underscores
                let display = name.trim_end_matches(".onnx").to_string();
                out.push((display, full));
            }
        } else if path.is_dir() {
            subdirs.push(path);
        }
    }
    for sub in subdirs {
        collect_onnx_recursive(&sub, depth - 1, out, seen);
    }
}

fn find_onnx_recursive(dir: &std::path::Path, depth: u8) -> Option<String> {
    if depth == 0 { return None; }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_owned();
        if path.is_file() && name.ends_with(".onnx") && !name.ends_with(".onnx.json") {
            return Some(path.display().to_string());
        } else if path.is_dir() {
            subdirs.push(path);
        }
    }
    for sub in subdirs {
        if let Some(found) = find_onnx_recursive(&sub, depth - 1) {
            return Some(found);
        }
    }
    None
}


/// Strip markdown so text reads naturally aloud.
fn strip_for_speech(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_code_block = false;

    for line in text.lines() {
        let trimmed = line.trim_start();

        // Toggle fenced code block — skip content inside
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block { continue; }

        // Strip heading markers
        let line = trimmed
            .trim_start_matches('#')
            .trim_start();

        // Strip leading list markers (-, *, +, or "1. 2." etc.)
        let line = if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).or_else(|| line.strip_prefix("+ ")) {
            rest
        } else {
            // numbered list: "1. " "12. "
            let maybe = line.trim_start_matches(|c: char| c.is_ascii_digit());
            if maybe.starts_with(". ") { maybe.trim_start_matches(". ") } else { line }
        };

        // Strip block quotes
        let line = line.strip_prefix("> ").unwrap_or(line);

        let line = strip_inline_md(line);
        let line = line.trim();
        if !line.is_empty() {
            out.push_str(line);
            out.push(' ');
        }
    }
    out.trim().to_string()
}

fn strip_inline_md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '`' => {
                // Skip inline code
                i += 1;
                while i < chars.len() && chars[i] != '`' { i += 1; }
            }
            '*' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                i += 1; // skip second *
            }
            '_' if i + 1 < chars.len() && chars[i + 1] == '_' => {
                i += 1; // skip second _
            }
            '*' | '_' => { /* skip single marker */ }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

/// Maximum words spoken per response — keeps TTS under ~90 seconds.
pub const TTS_WORD_LIMIT: usize = 200;

/// Default XTTS v2 speaker when no voice clone is configured.
pub const XTTS_DEFAULT_SPEAKER: &str = "Craig Gutsy";

/// Port for the XTTS v2 background server.
const XTTS_SERVER_PORT: u16 = 5002;

// ── CUDA detection ───────────────────────────────────────────────────────────

/// Check if an NVIDIA GPU with CUDA is available.
pub fn cuda_available() -> bool {
    which("nvidia-smi")
}

// ── XTTS v2 server lifecycle ─────────────────────────────────────────────────

/// Find the xtts-server.py script bundled with RustyClaw.
fn xtts_server_script() -> Option<PathBuf> {
    // Check relative to the running binary
    if let Ok(exe) = std::env::current_exe() {
        let exe_dir = exe.parent()?;
        // release binary: target/release/rustyclaw → ../../scripts/
        for candidate in &[
            exe_dir.join("../../scripts/xtts-server.py"),
            exe_dir.join("../scripts/xtts-server.py"),
            exe_dir.join("scripts/xtts-server.py"),
        ] {
            if candidate.exists() {
                return Some(candidate.canonicalize().ok()?);
            }
        }
    }
    // Check relative to CWD
    let cwd_script = PathBuf::from("scripts/xtts-server.py");
    if cwd_script.exists() {
        return cwd_script.canonicalize().ok();
    }
    None
}

/// Find the Python interpreter inside the TTS uv tool venv.
fn tts_python() -> Option<String> {
    // uv tool installs to ~/.local/share/uv/tools/tts/bin/python
    if let Some(home) = dirs::home_dir() {
        let uv_python = home.join(".local/share/uv/tools/tts/bin/python");
        if uv_python.exists() {
            return Some(uv_python.display().to_string());
        }
    }
    // Fallback: python3.11 in PATH
    if which("python3.11") { return Some("python3.11".into()); }
    None
}

/// Check if the XTTS v2 server is already running.
pub fn xtts_server_running() -> bool {
    std::net::TcpStream::connect(format!("127.0.0.1:{XTTS_SERVER_PORT}")).is_ok()
}

/// Start the XTTS v2 background server if not already running.
/// Returns Ok(port) on success. The server process is detached and persists
/// until RustyClaw exits or /voice speak off is called.
pub async fn ensure_xtts_server() -> Result<u16> {
    if xtts_server_running() {
        return Ok(XTTS_SERVER_PORT);
    }

    let script = xtts_server_script()
        .ok_or_else(|| anyhow!("xtts-server.py not found. Rebuild RustyClaw or check scripts/ dir."))?;
    let python = tts_python()
        .ok_or_else(|| anyhow!("No Python for TTS venv. Run: uv tool install TTS --python 3.11"))?;

    let mut args = vec![
        script.display().to_string(),
        XTTS_SERVER_PORT.to_string(),
    ];
    if !cuda_available() {
        args.push("--cpu".into());
    }

    // Spawn detached server process
    let _child = std::process::Command::new(&python)
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow!("Failed to start XTTS v2 server: {e}"))?;

    // Wait for server to become ready (up to 60s for model loading)
    for _ in 0..120 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if xtts_server_running() {
            return Ok(XTTS_SERVER_PORT);
        }
    }

    Err(anyhow!("XTTS v2 server failed to start within 60 seconds"))
}

/// Stop the XTTS v2 server if running.
pub fn stop_xtts_server() {
    let _ = std::process::Command::new("sh")
        .args(["-c", &format!("kill $(lsof -ti:{XTTS_SERVER_PORT}) 2>/dev/null")])
        .output();
}

// ── Server-based synthesis ───────────────────────────────────────────────────

/// Synthesise via the XTTS v2 server (fast — model stays loaded in GPU VRAM).
async fn speak_via_server(
    text: &str,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<bool> {
    let clean = strip_for_speech(text);
    if clean.is_empty() { return Ok(false); }

    let words: Vec<&str> = clean.split_whitespace().collect();
    let truncated = words.len() > TTS_WORD_LIMIT;
    let speech_text = if truncated {
        words[..TTS_WORD_LIMIT].join(" ") + ". Response trimmed."
    } else {
        clean
    };

    // Build JSON payload
    let clone_path = voice_clone_sample_path().filter(|p| p.exists());
    let body = if let Some(ref wav) = clone_path {
        format!(
            r#"{{"text":"{}","speaker_wav":"{}","language":"en"}}"#,
            speech_text.replace('\\', "\\\\").replace('"', "\\\""),
            wav.display().to_string().replace('\\', "\\\\").replace('"', "\\\""),
        )
    } else {
        format!(
            r#"{{"text":"{}","speaker":"{}","language":"en"}}"#,
            speech_text.replace('\\', "\\\\").replace('"', "\\\""),
            XTTS_DEFAULT_SPEAKER,
        )
    };

    let wav_out = std::env::temp_dir().join("rustyclaw-xtts-server.wav");
    tokio::pin!(stop_rx);

    // HTTP POST to server
    let mut curl = Command::new("curl")
        .args([
            "-s", "-X", "POST",
            &format!("http://127.0.0.1:{XTTS_SERVER_PORT}/tts"),
            "-H", "Content-Type: application/json",
            "-d", &body,
            "--output", &wav_out.display().to_string(),
            "--max-time", "30",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    tokio::select! {
        biased;
        _ = &mut stop_rx => {
            let _ = curl.kill().await;
            let _ = tokio::fs::remove_file(&wav_out).await;
            return Ok(truncated);
        }
        status = curl.wait() => {
            if !status?.success() {
                return Err(anyhow!("XTTS v2 server request failed"));
            }
        }
    }

    // Verify we got a real WAV (not an error page)
    let meta = tokio::fs::metadata(&wav_out).await?;
    if meta.len() < 1000 {
        let _ = tokio::fs::remove_file(&wav_out).await;
        return Err(anyhow!("XTTS v2 server returned invalid audio"));
    }

    play_wav(&wav_out, stop_rx).await?;
    Ok(truncated)
}

/// Synthesise `text` and play it.
///
/// Priority: XTTS v2 server (GPU, fast) → XTTS v2 CLI → piper (last resort).
/// `stop_rx` — send () to abort mid-synthesis or mid-playback.
/// Returns `Ok(true)` if truncated (hit word limit), `Ok(false)` if complete, `Err` on failure.
pub async fn speak(
    text: &str,
    _voice_model: Option<&str>,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<bool> {
    // ── Try XTTS v2 server first (fastest — model pre-loaded in VRAM) ──────
    if xtts_server_running() {
        return speak_via_server(text, stop_rx).await;
    }

    // ── XTTS v2 CLI fallback (cold start each call) ───────────────────────
    if xtts_available() {
        if let Some(clone_path) = voice_clone_sample_path() {
            if clone_path.exists() {
                return speak_cloned(text, &clone_path, stop_rx).await;
            }
        }
        return speak_xtts_default(text, stop_rx).await;
    }

    // ── Piper fallback (robotic, last resort) ──────────────────────────────
    speak_piper(text, _voice_model, stop_rx).await
}

/// Synthesise via XTTS v2 using a built-in default speaker (no clone needed).
async fn speak_xtts_default(
    text: &str,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<bool> {
    let clean = strip_for_speech(text);
    if clean.is_empty() { return Ok(false); }

    let words: Vec<&str> = clean.split_whitespace().collect();
    let truncated = words.len() > TTS_WORD_LIMIT;
    let speech_text = if truncated {
        words[..TTS_WORD_LIMIT].join(" ") + ". Response trimmed."
    } else {
        clean
    };

    let wav_out = std::env::temp_dir().join("rustyclaw-xtts-default.wav");
    let wav_out_str = wav_out.display().to_string();
    tokio::pin!(stop_rx);

    let mut cli_args = vec![
        "--model_name", "tts_models/multilingual/multi-dataset/xtts_v2",
        "--speaker_idx", XTTS_DEFAULT_SPEAKER,
        "--language_idx", "en",
        "--out_path", &wav_out_str,
        "--text", &speech_text,
    ];
    if cuda_available() {
        cli_args.extend(["--use_cuda", "true"]);
    }
    let mut tts_proc = Command::new("tts")
        .args(&cli_args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    tokio::select! {
        biased;
        _ = &mut stop_rx => {
            let _ = tts_proc.kill().await;
            let _ = tokio::fs::remove_file(&wav_out).await;
            return Ok(truncated);
        }
        status = tts_proc.wait() => {
            if !status?.success() {
                return Err(anyhow!(
                    "XTTS v2 synthesis failed.\n\
                     Install:  uv tool install TTS --python 3.11 --with 'transformers<4.46' --with 'torch<2.6' --with 'torchaudio<2.6'"
                ));
            }
        }
    }

    play_wav(&wav_out, stop_rx).await?;
    Ok(truncated)
}

/// Piper fallback — robotic but lightweight. Used only when XTTS v2 is not installed.
async fn speak_piper(
    text: &str,
    voice_model: Option<&str>,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<bool> {
    let clean = strip_for_speech(text);
    if clean.is_empty() { return Ok(false); }

    let words: Vec<&str> = clean.split_whitespace().collect();
    let truncated = words.len() > TTS_WORD_LIMIT;
    let speech_text = if truncated {
        words[..TTS_WORD_LIMIT].join(" ") + ". Response trimmed."
    } else {
        clean
    };

    let piper_bin = if which("piper") { "piper" }
        else if which("piper-tts") { "piper-tts" }
        else { return Err(anyhow!(
            "No TTS engine found.\n\n\
             Recommended (natural voice):  uv tool install TTS --python 3.11 --with 'transformers<4.46' --with 'torch<2.6' --with 'torchaudio<2.6'\n\
             Fallback (robotic):           pipx install piper-tts"
        )); };

    let model = voice_model
        .map(|s| s.to_string())
        .or_else(find_default_voice)
        .ok_or_else(|| anyhow!(
            "No piper voice model found.\n\
             mkdir -p ~/.local/share/piper && cd ~/.local/share/piper\n\
             wget …/en_US-lessac-high.onnx  (run /voice for the full URL)"
        ))?;

    let wav_out = std::env::temp_dir().join("rustyclaw-tts.wav");
    tokio::pin!(stop_rx);

    let mut piper = Command::new(piper_bin)
        .args(["--model", &model, "--output_file", &wav_out.display().to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = piper.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(speech_text.as_bytes()).await?;
    }

    tokio::select! {
        biased;
        _ = &mut stop_rx => {
            let _ = piper.kill().await;
            let _ = tokio::fs::remove_file(&wav_out).await;
            return Ok(truncated);
        }
        _ = piper.wait() => {}
    }

    play_wav(&wav_out, stop_rx).await?;
    Ok(truncated)
}

/// Play a WAV file through the first available audio player, then clean up.
async fn play_wav(
    wav_path: &std::path::Path,
    mut stop_rx: std::pin::Pin<&mut tokio::sync::oneshot::Receiver<()>>,
) -> Result<()> {
    let path_str = wav_path.display().to_string();
    let players: &[(&str, &[&str])] = &[
        ("aplay",  &["-q"]),
        ("paplay", &[]),
        ("mpv",    &["--really-quiet", "--no-video"]),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
        ("play",   &["-q"]),
    ];
    let mut played = false;
    for (player, args) in players {
        if which(player) {
            let mut child = Command::new(player)
                .args(*args)
                .arg(&path_str)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            tokio::select! {
                biased;
                _ = stop_rx.as_mut() => { let _ = child.kill().await; }
                _ = child.wait() => {}
            }
            played = true;
            break;
        }
    }
    let _ = tokio::fs::remove_file(wav_path).await;

    if !played {
        return Err(anyhow!(
            "No audio player found.\n\
             Install:  sudo pacman -S alsa-utils   # provides aplay\n\
             Or:       sudo pacman -S mpv"
        ));
    }
    Ok(())
}

// ── Status display ────────────────────────────────────────────────────────────

pub fn voice_status(enabled: bool, tts_enabled: bool) -> String {
    let recorder = find_recorder();
    let rec_status = match &recorder {
        Some(RecorderBackend::Arecord) => "✓ arecord",
        Some(RecorderBackend::Sox)     => "✓ sox",
        Some(RecorderBackend::Ffmpeg)  => "✓ ffmpeg",
        None => "✗ no recorder (install: sudo pacman -S alsa-utils)",
    };

    let whisper = if local_whisper_available() {
        "✓ local whisper (offline)"
    } else {
        "✗ whisper not installed (pipx install openai-whisper)"
    };

    let api_key_status = match voice_api_key() {
        Some(_) => "✓ API key found".to_string(),
        None    => "✗ no API key — add OPENAI_API_KEY=sk-... to ~/.env".to_string(),
    };

    let transcription_ok = local_whisper_available() || voice_api_key().is_some();
    let recorder_ok = recorder.is_some();

    let input_overall = if enabled {
        if recorder_ok && transcription_ok {
            "ENABLED  ● Ready — press Ctrl+R to record"
        } else {
            "ENABLED  ⚠ Setup incomplete (see below)"
        }
    } else {
        "DISABLED"
    };

    // ── TTS status — XTTS v2 is primary, piper is last-resort fallback ─────
    let xtts_ok = xtts_available();
    let piper_ok = piper_available();
    let player_ok = audio_player_available();
    let tts_overall = if tts_enabled { "ENABLED" } else { "DISABLED" };

    let server_up = xtts_server_running();
    let gpu = cuda_available();
    let tts_engine = if xtts_ok && server_up {
        if gpu { "✓ XTTS v2 server running (GPU — fast)" } else { "✓ XTTS v2 server running (CPU)" }
    } else if xtts_ok {
        "✓ XTTS v2 available (server will auto-start on use)"
    } else if piper_ok {
        "⚠ piper only — robotic fallback (install XTTS v2 for natural voice)"
    } else {
        "✗ no TTS engine installed"
    };

    // Voice clone status
    let clone_path = voice_clone_sample_path();
    let clone_exists = clone_path.as_ref().map_or(false, |p| p.exists());

    let clone_status = if clone_exists {
        let tier = detect_clone_tier(&clone_path.unwrap());
        format!("✓ your voice ({tier} tier)")
    } else if xtts_ok {
        format!("default speaker ({XTTS_DEFAULT_SPEAKER})")
    } else {
        "— (requires XTTS v2)".to_string()
    };

    let tts_ready = xtts_ok || piper_ok;
    let all_input_ok = recorder_ok && transcription_ok;

    let mut out = format!(
        "Voice Input  {input_overall}\n\
         \n\
         Audio capture:      {rec_status}\n\
         Transcription:      {whisper}\n\
         API key:            {api_key_status}\n\
         \n\
         TTS output  {tts_overall}\n\
         \n\
         Engine:             {tts_engine}\n\
         Voice:              {clone_status}\n\
         Audio player:       {}\n\
         \n\
         Commands:\n\
           /voice enable         — enable voice input (Ctrl+R to record)\n\
           /voice disable        — disable voice input\n\
           /voice speak on|off   — enable/disable TTS output\n\
           /voice test           — play a test phrase\n\
           /voice clone          — clone your voice (XTTS v2)\n\
           /voice clone remove   — revert to default speaker\n\
           Ctrl+R                — start/stop recording (when enabled)",
        if player_ok { "✓ available" } else { "✗ no player (install: sudo pacman -S mpv)" },
    );

    // Install instructions for missing components
    if all_input_ok && tts_ready && player_ok {
        out.push_str("\n\n  ✓ All set — voice input and TTS fully configured.");
    } else {
        if !recorder_ok {
            out.push_str("\n\n  Setup needed — audio capture:\n\
                           \n    sudo pacman -S alsa-utils");
        }
        if !transcription_ok {
            out.push_str("\n\n  Setup needed — transcription:\n\
                           \n    pipx install openai-whisper    # offline\n\
                           \n    — or: echo 'OPENAI_API_KEY=sk-...' >> ~/.env");
        }
        if !xtts_ok {
            out.push_str("\n\n  Setup needed — XTTS v2 (natural voice, STRONGLY recommended):\n\
                           \n    uv tool install TTS --python 3.11 \\\n\
                             \x20     --with 'transformers<4.46' --with 'torch<2.6' --with 'torchaudio<2.6'\n\
                           \n    ⚠ Piper is a robotic fallback. XTTS v2 sounds like a real human.");
        }
        if !player_ok {
            out.push_str("\n\n  Setup needed — audio player:\n\
                           \n    sudo pacman -S mpv             # or alsa-utils for aplay");
        }
    }

    out
}

// ── Voice cloning via XTTS v2 ────────────────────────────────────────────────

/// Clone quality tiers — determines recording duration and guidance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CloneTier {
    /// 10 seconds — recognizable but synthetic
    Quick,
    /// 60 seconds — natural rhythm, occasional artifacts
    Recommended,
    /// 5+ minutes — near-perfect clone
    Premium,
}

impl CloneTier {
    pub fn label(&self) -> &'static str {
        match self {
            CloneTier::Quick       => "quick",
            CloneTier::Recommended => "recommended",
            CloneTier::Premium     => "premium",
        }
    }

    pub fn duration_secs(&self) -> u64 {
        match self {
            CloneTier::Quick       => 10,
            CloneTier::Recommended => 60,
            CloneTier::Premium     => 300,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            CloneTier::Quick       => "10 seconds — recognizable you, but clearly synthetic",
            CloneTier::Recommended => "60 seconds — natural rhythm, good quality (guided prompts)",
            CloneTier::Premium     => "5+ minutes — near-perfect voice clone",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "quick" | "1" | "10s"      => Some(CloneTier::Quick),
            "recommended" | "2" | "60s" => Some(CloneTier::Recommended),
            "premium" | "3" | "5m"      => Some(CloneTier::Premium),
            _ => None,
        }
    }
}

impl std::fmt::Display for CloneTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Check if XTTS v2 is available (Coqui TTS Python package).
pub fn xtts_available() -> bool {
    which("tts")
}

/// Directory where voice clone samples are stored.
pub fn voice_clone_dir() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".local/share/rustyclaw/voice-clone"))
}

/// Path to the active voice clone WAV sample.
pub fn voice_clone_sample_path() -> Option<std::path::PathBuf> {
    voice_clone_dir().map(|d| d.join("my-voice.wav"))
}

/// Detect which tier a recorded sample falls into based on file duration.
fn detect_clone_tier(wav_path: &std::path::Path) -> &'static str {
    // Estimate duration from file size: 16-bit mono 22050 Hz ≈ 44100 bytes/sec
    let size = std::fs::metadata(wav_path).map(|m| m.len()).unwrap_or(0);
    let est_secs = size / 44100;
    if est_secs >= 240 { "premium" }
    else if est_secs >= 40 { "recommended" }
    else { "quick" }
}

/// Guided reading prompts for the recommended tier.
/// Designed to exercise varied phonemes, prosody, questions, and exclamations.
pub const GUIDED_PROMPTS: &[&str] = &[
    "The quick brown fox jumps over the lazy dog. Every letter matters when you're building a voice.",
    "How does this function handle edge cases? I think we need to refactor the error path.",
    "That's a great idea! Let me check if the tests pass before we merge this pull request.",
    "The server responded with a five hundred error. We should add retry logic with exponential backoff.",
    "Why would anyone use a linked list here? An array would be so much faster for sequential access.",
    "Perfect. Ship it. The benchmarks look incredible — forty percent faster than the previous version.",
    "I'm not sure about this approach. Could we try something simpler first? Maybe a hash map instead.",
    "Documentation is important, but working code is more important. Let's fix the bug, then write docs.",
];

/// Build instructions text for recording at a given tier.
pub fn recording_instructions(tier: CloneTier) -> String {
    let duration = tier.duration_secs();
    let mut text = format!(
        "Voice Clone Recording — {} tier ({} seconds)\n\n\
         Tips for best quality:\n\
         • Use a quiet room — no background noise, fans, or music\n\
         • Speak at your normal pace and volume\n\
         • Hold your mic 6-12 inches from your mouth\n\
         • Vary your tone naturally — don't read in monotone\n\n",
        tier.label(),
        duration,
    );

    match tier {
        CloneTier::Quick => {
            text.push_str(
                "Read this aloud when recording starts:\n\n\
                 \"The quick brown fox jumps over the lazy dog.\n\
                 Every letter matters when you're building a voice.\"\n\n\
                 Press Ctrl+R to start recording. Press Ctrl+R again to stop after ~10 seconds.",
            );
        }
        CloneTier::Recommended => {
            text.push_str("Read these prompts aloud, one after another:\n\n");
            for (i, prompt) in GUIDED_PROMPTS.iter().enumerate() {
                text.push_str(&format!("  {}. \"{}\"\n\n", i + 1, prompt));
            }
            text.push_str(
                "Press Ctrl+R to start recording. Read all prompts naturally, then press Ctrl+R to stop.",
            );
        }
        CloneTier::Premium => {
            text.push_str(
                "For premium quality, read continuously for 5+ minutes.\n\n\
                 Suggestions:\n\
                 • Read a README or documentation file from this project aloud\n\
                 • Narrate what you're working on — explain your code\n\
                 • Read a blog post or article that interests you\n\
                 • Just talk naturally about anything\n\n\
                 The longer and more varied your speech, the better the clone.\n\n\
                 Press Ctrl+R to start recording. Press Ctrl+R again when done (aim for 5+ minutes).",
            );
        }
    }
    text
}

/// Save a recorded WAV file as the voice clone sample.
/// Copies from the temp recording location to the voice clone directory.
pub async fn save_voice_clone(tier: CloneTier) -> Result<String> {
    let src = temp_wav_path();
    if !src.exists() {
        return Err(anyhow!("No recording found. Record with Ctrl+R first."));
    }

    // Validate minimum duration
    let size = tokio::fs::metadata(&src).await?.len();
    let est_secs = size / 44100; // rough estimate for 16-bit mono 22050Hz
    let min_secs = match tier {
        CloneTier::Quick       => 3,
        CloneTier::Recommended => 20,
        CloneTier::Premium     => 120,
    };
    if est_secs < min_secs {
        return Err(anyhow!(
            "Recording too short (~{}s). {} tier needs at least {}s. Try again.",
            est_secs, tier.label(), min_secs,
        ));
    }

    let dest_dir = voice_clone_dir()
        .ok_or_else(|| anyhow!("Cannot determine home directory"))?;
    tokio::fs::create_dir_all(&dest_dir).await?;

    let dest = dest_dir.join("my-voice.wav");
    tokio::fs::copy(&src, &dest).await?;

    // Also save the tier info
    let meta = dest_dir.join("meta.txt");
    tokio::fs::write(&meta, format!("tier={}\nsize={}\n", tier.label(), size)).await?;

    Ok(format!(
        "Voice clone saved ({} tier, ~{}s).\n\
         Location: {}\n\n\
         TTS will now use XTTS v2 with your voice.\n\
         Use /voice clone remove to revert to the default XTTS v2 speaker.",
        tier.label(), est_secs, dest.display(),
    ))
}

/// Remove the voice clone sample, reverting to XTTS v2 default speaker.
pub async fn remove_voice_clone() -> Result<String> {
    let dir = voice_clone_dir()
        .ok_or_else(|| anyhow!("Cannot determine home directory"))?;
    let sample = dir.join("my-voice.wav");
    if sample.exists() {
        tokio::fs::remove_file(&sample).await?;
        let meta = dir.join("meta.txt");
        let _ = tokio::fs::remove_file(&meta).await;
        Ok(format!("Voice clone removed. TTS reverted to XTTS v2 default speaker ({XTTS_DEFAULT_SPEAKER})."))
    } else {
        Ok("No voice clone configured.".into())
    }
}

/// Synthesise `text` using XTTS v2 with the user's cloned voice.
pub async fn speak_cloned(
    text: &str,
    clone_wav: &std::path::Path,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<bool> {
    let clean = strip_for_speech(text);
    if clean.is_empty() { return Ok(false); }

    let words: Vec<&str> = clean.split_whitespace().collect();
    let truncated = words.len() > TTS_WORD_LIMIT;
    let speech_text = if truncated {
        words[..TTS_WORD_LIMIT].join(" ") + ". Response trimmed."
    } else {
        clean
    };

    let wav_out = std::env::temp_dir().join("rustyclaw-xtts.wav");
    let wav_out_str = wav_out.display().to_string();
    tokio::pin!(stop_rx);

    let clone_str = clone_wav.display().to_string();
    let mut cli_args = vec![
        "--model_name", "tts_models/multilingual/multi-dataset/xtts_v2",
        "--speaker_wav", &clone_str,
        "--language_idx", "en",
        "--out_path", &wav_out_str,
        "--text", &speech_text,
    ];
    if cuda_available() {
        cli_args.extend(["--use_cuda", "true"]);
    }
    let mut tts_proc = Command::new("tts")
        .args(&cli_args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    tokio::select! {
        biased;
        _ = &mut stop_rx => {
            let _ = tts_proc.kill().await;
            let _ = tokio::fs::remove_file(&wav_out).await;
            return Ok(truncated);
        }
        status = tts_proc.wait() => {
            if !status?.success() {
                return Err(anyhow!(
                    "XTTS v2 synthesis failed.\n\
                     Install:  uv tool install TTS --python 3.11 --with 'transformers<4.46' --with 'torch<2.6' --with 'torchaudio<2.6'"
                ));
            }
        }
    }

    play_wav(&wav_out, stop_rx).await?;
    Ok(truncated)
}
