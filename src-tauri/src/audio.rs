//! 音效播放與語音播報。
//!
//! 獨立執行緒負責播放，避免阻塞計時器。語音使用 Windows 內建語音合成
//! （System.Speech），由一個常駐的隱藏 PowerShell 程序逐行朗讀。

use std::io::Cursor;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::OnceLock;

pub const BEEP1: &[u8] = include_bytes!("../sounds/beep1.wav");
pub const BEEP2: &[u8] = include_bytes!("../sounds/beep2.wav");
pub const BEEP3: &[u8] = include_bytes!("../sounds/beep3.wav");

enum Cmd {
    Bytes(&'static [u8]),
    File(String),
    Speak(String),
}

static TX: OnceLock<Sender<Cmd>> = OnceLock::new();

pub fn init() {
    let (tx, rx) = mpsc::channel();
    let _ = TX.set(tx);
    std::thread::spawn(move || worker(rx));
}

/// 依計時器的音效設定播放
pub fn play(sound: &str, file: &str, name: &str) {
    let cmd = match sound {
        "beep2" => Cmd::Bytes(BEEP2),
        "beep3" => Cmd::Bytes(BEEP3),
        "tts" if !name.trim().is_empty() => Cmd::Speak(name.to_string()),
        "file" if !file.is_empty() => Cmd::File(file.to_string()),
        _ => Cmd::Bytes(BEEP1),
    };
    if let Some(tx) = TX.get() {
        let _ = tx.send(cmd);
    }
}

fn worker(rx: Receiver<Cmd>) {
    // 輸出裝置必須留在同一個執行緒；沒有音效裝置時就靜默略過
    let output = rodio::OutputStream::try_default().ok();
    let mut tts = tts::Speaker::default();

    let play_bytes = |bytes: Vec<u8>| {
        let Some((_, handle)) = output.as_ref() else { return };
        let Ok(sink) = rodio::Sink::try_new(handle) else { return };
        match rodio::Decoder::new(Cursor::new(bytes)) {
            Ok(src) => {
                sink.append(src);
                sink.detach();
            }
            Err(e) => eprintln!("無法解碼音效：{e}"),
        }
    };

    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Bytes(b) => play_bytes(b.to_vec()),
            Cmd::File(path) => match std::fs::read(&path) {
                Ok(b) => play_bytes(b),
                Err(_) => play_bytes(BEEP1.to_vec()),
            },
            Cmd::Speak(text) => tts.speak(&text),
        }
    }
}

#[cfg(windows)]
mod tts {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // 優先使用繁體中文語音，其次任何中文語音
    const SCRIPT: &str = r#"
[Console]::InputEncoding = [Text.Encoding]::UTF8
Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
try {
  $v = $s.GetInstalledVoices() | ForEach-Object { $_.VoiceInfo } |
       Where-Object { $_.Culture.Name -like 'zh*' } |
       Sort-Object { if ($_.Culture.Name -eq 'zh-TW') { 0 } else { 1 } } |
       Select-Object -First 1
  if ($v) { $s.SelectVoice($v.Name) }
} catch {}
while (($l = [Console]::In.ReadLine()) -ne $null) { if ($l) { $s.Speak($l) } }
"#;

    #[derive(Default)]
    pub struct Speaker {
        child: Option<Child>,
    }

    impl Speaker {
        fn alive(&mut self) -> bool {
            matches!(self.child.as_mut().map(|c| c.try_wait()), Some(Ok(None)))
        }

        pub fn speak(&mut self, text: &str) {
            if !self.alive() {
                self.child = Command::new("powershell")
                    .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn()
                    .ok();
            }
            if let Some(stdin) = self.child.as_mut().and_then(|c| c.stdin.as_mut()) {
                let line = text.replace(['\r', '\n'], " ");
                let _ = writeln!(stdin, "{line}");
                let _ = stdin.flush();
            }
        }
    }
}

#[cfg(not(windows))]
mod tts {
    #[derive(Default)]
    pub struct Speaker;
    impl Speaker {
        pub fn speak(&mut self, _text: &str) {}
    }
}
