//! 音效播放與語音播報（音量 0–200%）。
//!
//! 獨立執行緒負責播放，避免阻塞計時器。語音使用 Windows 內建語音合成（程式直接呼叫），
//! 把文字合成成 WAV 後和提示音一樣經由同一個播放器輸出，所以音量設定對語音也有效。

use std::io::Cursor;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::OnceLock;

pub const BEEP1: &[u8] = include_bytes!("../sounds/beep1.wav");
pub const BEEP2: &[u8] = include_bytes!("../sounds/beep2.wav");
pub const BEEP3: &[u8] = include_bytes!("../sounds/beep3.wav");

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

/// 播放音效。sound："beep1" | "beep2" | "beep3" | "tts" | "file"；volume：0–200
pub fn play(sound: &str, file: &str, name: &str, volume: u32) {
    let src = match sound {
        "beep2" => Src::Bytes(BEEP2),
        "beep3" => Src::Bytes(BEEP3),
        "tts" if !name.trim().is_empty() => Src::Speak(name.to_string()),
        "file" if !file.is_empty() => Src::File(file.to_string()),
        _ => Src::Bytes(BEEP1),
    };
    let volume = volume.min(200) as f32 / 100.0;
    if let Some(tx) = TX.get() {
        let _ = tx.send(Cmd { src, volume });
    }
}

/// 目前開著的輸出裝置
struct Out {
    name: String,
    _stream: rodio::OutputStream,
    handle: rodio::OutputStreamHandle,
}

/// 每次播放前用當下的預設輸出裝置：換了耳機／喇叭，或之前沒有音效裝置（例如開機時藍牙耳機還沒連上）就重新開
fn current_output(out: &mut Option<Out>, reopen: bool) -> Option<&rodio::OutputStreamHandle> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let Some(dev) = rodio::cpal::default_host().default_output_device() else {
        *out = None;
        return None;
    };
    let name = dev.name().unwrap_or_default();
    if reopen || out.as_ref().map_or(true, |o| o.name != name) {
        *out = None; // 先關掉舊的裝置
        *out = rodio::OutputStream::try_from_device(&dev).ok().map(|(s, handle)| Out { name, _stream: s, handle });
    }
    out.as_ref().map(|o| &o.handle)
}

fn play_bytes(out: &mut Option<Out>, bytes: Vec<u8>, volume: f32) {
    let sink = match current_output(out, false).map(rodio::Sink::try_new) {
        Some(Ok(sink)) => sink,
        // 裝置失效：重新開一次再試
        _ => match current_output(out, true).map(rodio::Sink::try_new) {
            Some(Ok(sink)) => sink,
            _ => return,
        },
    };
    match rodio::Decoder::new(Cursor::new(bytes)) {
        Ok(src) => {
            sink.set_volume(volume);
            sink.append(src);
            sink.detach();
        }
        Err(e) => eprintln!("無法解碼音效：{e}"),
    }
}

fn worker(rx: Receiver<Cmd>) {
    // 輸出裝置必須留在同一個執行緒；沒有音效裝置時就靜默略過
    let mut out: Option<Out> = None;
    let mut tts = tts::Speaker::default();

    while let Ok(Cmd { src, volume }) = rx.recv() {
        match src {
            Src::Bytes(b) => play_bytes(&mut out, b.to_vec(), volume),
            Src::File(path) => match std::fs::read(&path) {
                Ok(b) => play_bytes(&mut out, b, volume),
                Err(_) => play_bytes(&mut out, BEEP1.to_vec(), volume),
            },
            Src::Speak(text) => match tts.synthesize(&text) {
                Some(wav) => play_bytes(&mut out, wav, volume),
                None => play_bytes(&mut out, BEEP1.to_vec(), volume),
            },
        }
    }
}

/// Windows 內建的語音合成：程式直接呼叫（不開 PowerShell、不寫暫存檔），合成成 WAV 再交給播放器
#[cfg(windows)]
mod tts {
    use windows::core::HSTRING;
    use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceInformation};
    use windows::Storage::Streams::DataReader;

    #[derive(Default)]
    pub struct Speaker {
        synth: Option<SpeechSynthesizer>,
    }

    /// 優先用繁體中文語音，其次任何中文語音；都沒有就用系統預設的語音
    fn pick_voice() -> Option<VoiceInformation> {
        let mut best: Option<(u8, VoiceInformation)> = None;
        for v in SpeechSynthesizer::AllVoices().ok()? {
            let lang = v.Language().map(|l| l.to_string().to_ascii_lowercase()).unwrap_or_default();
            let rank = if lang == "zh-tw" {
                0
            } else if lang.starts_with("zh") {
                1
            } else {
                continue;
            };
            if best.as_ref().map_or(true, |(r, _)| rank < *r) {
                best = Some((rank, v));
            }
        }
        best.map(|(_, v)| v)
    }

    fn speak(synth: &SpeechSynthesizer, text: &str) -> windows::core::Result<Vec<u8>> {
        let stream = synth.SynthesizeTextToStreamAsync(&HSTRING::from(text))?.get()?;
        let size = stream.Size()? as u32;
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
        reader.LoadAsync(size)?.get()?;
        let mut wav = vec![0u8; size as usize];
        reader.ReadBytes(&mut wav)?;
        Ok(wav)
    }

    impl Speaker {
        /// 把文字合成成 WAV，回傳檔案內容
        pub fn synthesize(&mut self, text: &str) -> Option<Vec<u8>> {
            if self.synth.is_none() {
                let synth = SpeechSynthesizer::new().ok()?;
                if let Some(v) = pick_voice() {
                    let _ = synth.SetVoice(&v);
                }
                self.synth = Some(synth);
            }
            let line = text.replace(['\r', '\n'], " ");
            match speak(self.synth.as_ref()?, &line) {
                Ok(wav) if !wav.is_empty() => Some(wav),
                // 失敗時下次重新建立
                _ => {
                    self.synth = None;
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
