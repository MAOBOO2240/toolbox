//! 音效播放與語音播報（音量 0–200%）。
//!
//! 獨立執行緒負責播放，避免阻塞計時器。語音使用 Windows 內建語音合成
//! （System.Speech）：常駐一個隱藏的 PowerShell 把文字合成成 WAV，
//! 再和提示音一樣經由同一個播放器輸出，所以音量設定對語音也有效。

use std::io::Cursor;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::OnceLock;

pub const BEEP1: &[u8] = include_bytes!("../sounds/beep1.wav");
pub const BEEP2: &[u8] = include_bytes!("../sounds/beep2.wav");
pub const BEEP3: &[u8] = include_bytes!("../sounds/beep3.wav");
/// 撞到木櫃：兩下低沉的「咚、咚」
pub const THUD: &[u8] = include_bytes!("../sounds/thud.wav");

enum Src {
    Bytes(&'static [u8]),
    File(String),
    Speak(String),
}

struct Cmd {
    src: Src,
    /// 1.0 = 100%
    volume: f32,
}

static TX: OnceLock<Sender<Cmd>> = OnceLock::new();

pub fn init() {
    let (tx, rx) = mpsc::channel();
    let _ = TX.set(tx);
    std::thread::spawn(move || worker(rx));
}

/// 播放音效。sound："beep1" | "beep2" | "beep3" | "thud" | "tts" | "file"；volume：0–200
pub fn play(sound: &str, file: &str, name: &str, volume: u32) {
    let src = match sound {
        "beep2" => Src::Bytes(BEEP2),
        "beep3" => Src::Bytes(BEEP3),
        "thud" => Src::Bytes(THUD),
        "tts" if !name.trim().is_empty() => Src::Speak(name.to_string()),
        "file" if !file.is_empty() => Src::File(file.to_string()),
        _ => Src::Bytes(BEEP1),
    };
    let volume = volume.min(200) as f32 / 100.0;
    if let Some(tx) = TX.get() {
        let _ = tx.send(Cmd { src, volume });
    }
}

fn worker(rx: Receiver<Cmd>) {
    // 輸出裝置必須留在同一個執行緒；沒有音效裝置時就靜默略過
    let output = rodio::OutputStream::try_default().ok();
    let mut tts = tts::Speaker::default();

    let play_bytes = |bytes: Vec<u8>, volume: f32| {
        let Some((_, handle)) = output.as_ref() else { return };
        let Ok(sink) = rodio::Sink::try_new(handle) else { return };
        match rodio::Decoder::new(Cursor::new(bytes)) {
            Ok(src) => {
                sink.set_volume(volume);
                sink.append(src);
                sink.detach();
            }
            Err(e) => eprintln!("無法解碼音效：{e}"),
        }
    };

    while let Ok(Cmd { src, volume }) = rx.recv() {
        match src {
            Src::Bytes(b) => play_bytes(b.to_vec(), volume),
            Src::File(path) => match std::fs::read(&path) {
                Ok(b) => play_bytes(b, volume),
                Err(_) => play_bytes(BEEP1.to_vec(), volume),
            },
            Src::Speak(text) => match tts.synthesize(&text) {
                Some(wav) => play_bytes(wav, volume),
                None => play_bytes(BEEP1.to_vec(), volume),
            },
        }
    }
}

#[cfg(windows)]
mod tts {
    use std::io::{BufRead, BufReader, Write};
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    struct Proc {
        child: Child,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
    }

    #[derive(Default)]
    pub struct Speaker {
        proc: Option<Proc>,
    }

    fn wav_path() -> PathBuf {
        std::env::temp_dir().join("toolbox_tts.wav")
    }

    fn spawn() -> Option<Proc> {
        let path = wav_path().to_string_lossy().replace('\'', "''");
        // 優先使用繁體中文語音，其次任何中文語音；每行文字合成一個 WAV 後回覆 ok
        let script = format!(
            r#"
[Console]::InputEncoding = [Text.Encoding]::UTF8
Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
try {{
  $v = $s.GetInstalledVoices() | ForEach-Object {{ $_.VoiceInfo }} |
       Where-Object {{ $_.Culture.Name -like 'zh*' }} |
       Sort-Object {{ if ($_.Culture.Name -eq 'zh-TW') {{ 0 }} else {{ 1 }} }} |
       Select-Object -First 1
  if ($v) {{ $s.SelectVoice($v.Name) }}
}} catch {{}}
$p = '{path}'
while (($l = [Console]::In.ReadLine()) -ne $null) {{
  try {{ $s.SetOutputToWaveFile($p); $s.Speak($l) }} catch {{}}
  try {{ $s.SetOutputToNull() }} catch {{}}
  [Console]::Out.WriteLine('ok'); [Console]::Out.Flush()
}}
"#
        );
        let mut child = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = BufReader::new(child.stdout.take()?);
        Some(Proc { child, stdin, stdout })
    }

    impl Speaker {
        /// 把文字合成成 WAV，回傳檔案內容
        pub fn synthesize(&mut self, text: &str) -> Option<Vec<u8>> {
            let alive = matches!(self.proc.as_mut().map(|p| p.child.try_wait()), Some(Ok(None)));
            if !alive {
                self.proc = spawn();
            }
            let p = self.proc.as_mut()?;
            let line = text.replace(['\r', '\n'], " ");
            if writeln!(p.stdin, "{line}").and_then(|_| p.stdin.flush()).is_err() {
                self.proc = None;
                return None;
            }
            let mut reply = String::new();
            match p.stdout.read_line(&mut reply) {
                Ok(n) if n > 0 => std::fs::read(wav_path()).ok(),
                _ => {
                    self.proc = None;
                    None
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod tts {
    #[derive(Default)]
    pub struct Speaker;
    impl Speaker {
        pub fn synthesize(&mut self, _text: &str) -> Option<Vec<u8>> {
            None
        }
    }
}
