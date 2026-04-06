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

/// Synthesise `text` with piper and play it.
/// `stop_rx` — send () to abort mid-synthesis or mid-playback.
/// Returns `Ok(true)` if truncated (hit word limit), `Ok(false)` if complete, `Err` on failure.
pub async fn speak(
    text: &str,
    voice_model: Option<&str>,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<bool> {
    let clean = strip_for_speech(text);
    if clean.is_empty() { return Ok(false); }

    // Enforce word limit — never synthesise a wall of text
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
            "piper not found.\n\
             Install:  pip install piper-tts\n\
             Binary:   https://github.com/rhasspy/piper/releases"
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

    // ── Step 1: Synthesise via piper ────────────────────────────────────────
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

    // ── Step 2: Play the WAV ────────────────────────────────────────────────
    let path_str = wav_out.display().to_string();
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
                _ = &mut stop_rx => { let _ = child.kill().await; }
                _ = child.wait() => {}
            }
            played = true;
            break;
        }
    }
    let _ = tokio::fs::remove_file(&wav_out).await;

    if !played {
        return Err(anyhow!(
            "No audio player found.\n\
             Install one:  sudo apt install alsa-utils    # provides aplay\n\
             Or:           sudo apt install mpv\n\
             Or:           sudo apt install ffmpeg        # provides ffplay"
        ));
    }
    Ok(truncated)
}

// ── Status display ────────────────────────────────────────────────────────────

pub fn voice_status(enabled: bool, tts_enabled: bool) -> String {
    let recorder = find_recorder();
    let rec_status = match &recorder {
        Some(RecorderBackend::Arecord) => "✓ arecord (available)",
        Some(RecorderBackend::Sox)     => "✓ sox (available)",
        Some(RecorderBackend::Ffmpeg)  => "✓ ffmpeg (available)",
        None => "✗ no recorder found (install: sudo apt install alsa-utils)",
    };

    let whisper = if local_whisper_available() {
        "✓ local whisper (offline transcription)"
    } else {
        "✗ whisper not installed (pip install openai-whisper)"
    };

    let api_key_status = match voice_api_key() {
        Some(_) => "✓ API key found (OPENAI_API_KEY / WHISPER_API_KEY)".to_string(),
        None    => {
            // Check if a .env exists in CWD or home so we can give targeted advice
            let has_cwd_env = std::env::current_dir()
                .map(|p| p.join(".env").exists())
                .unwrap_or(false);
            let has_home_env = dirs::home_dir()
                .map(|p| p.join(".env").exists())
                .unwrap_or(false);
            if has_cwd_env || has_home_env {
                "✗ no API key — add OPENAI_API_KEY=sk-... to your .env file".to_string()
            } else {
                "✗ no API key — create ~/.env with: OPENAI_API_KEY=sk-...".to_string()
            }
        }
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

    let tts_piper = if piper_available() { "✓ piper (available)" } else { "✗ piper not found" };
    let tts_model = match find_default_voice() {
        Some(p) => format!("✓ voice model: {p}"),
        None    => "✗ no voice model — download en_US-lessac-high.onnx to ~/.local/share/piper/".to_string(),
    };
    let tts_overall = if tts_enabled { "ENABLED" } else { "DISABLED" };

    let piper_ok = piper_available();
    let model_ok = find_default_voice().is_some();
    let player_ok = audio_player_available();
    let all_input_ok = recorder_ok && transcription_ok;
    let all_tts_ok = piper_ok && model_ok && player_ok;

    let mut out = format!(
        "Voice Input  {input_overall}\n\
         \n\
         Audio capture:\n\
           {rec_status}\n\
         \n\
         Transcription (speech → text):\n\
           {whisper}\n\
           {api_key_status}\n\
         \n\
         TTS output (text → speech)  {tts_overall}\n\
           {tts_piper}\n\
           {tts_model}\n\
         \n\
         Commands:\n\
           /voice enable         — enable voice input (Ctrl+R to record)\n\
           /voice disable        — disable voice input\n\
           /voice speak on|off   — enable/disable TTS response output\n\
           /voice model          — pick a voice model\n\
           Ctrl+R                — start/stop recording (when enabled)"
    );

    // Only show install instructions for things that are actually missing
    if all_input_ok && all_tts_ok {
        out.push_str("\n\n  ✓ All set — voice input and TTS are fully configured.");
    } else {
        if !recorder_ok {
            out.push_str("\n\n  Setup needed — audio capture:\n\
                           \n    sudo apt install alsa-utils    # provides arecord");
        }
        if !transcription_ok {
            out.push_str("\n\n  Setup needed — transcription:\n\
                           \n    pip install openai-whisper     # offline transcription\n\
                           \n    — or set API key:\n\
                           \n    echo 'OPENAI_API_KEY=sk-...' >> ~/.env");
        }
        if !piper_ok {
            out.push_str("\n\n  Setup needed — piper TTS:\n\
                           \n    pip install piper-tts          # or download from GitHub");
        }
        if !model_ok {
            out.push_str("\n\n  Setup needed — voice model:\n\
                           \n    mkdir -p ~/.local/share/piper && cd ~/.local/share/piper\n\
                           \n    wget https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/en/en_US/lessac/high/en_US-lessac-high.onnx\n\
                           \n    wget https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/en/en_US/lessac/high/en_US-lessac-high.onnx.json");
        }
        if !player_ok {
            out.push_str("\n\n  Setup needed — audio player:\n\
                           \n    sudo apt install alsa-utils    # provides aplay");
        }
    }

    out
}
