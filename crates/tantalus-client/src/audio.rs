//! Sound for the play window.
//!
//! One thread owns everything: it loads the sound bank, runs the [`AudioDirector`] (the ported
//! sound system) and keeps a short buffer of rendered stereo samples ahead of the sound card,
//! whose callback only copies from that buffer. The window sends [`AudioCue`]s over a channel and
//! never waits for audio. Nothing here runs in the capture modes: `capture-play` has no audio
//! unless `--audio-log` asks for a printout of the cues, which opens no device.

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};

use tantalus_tools::play_audio::AudioCue;
use tantalus_tools::sound_bank::SoundBank;

/// A request to the audio thread.
enum Message {
    Cues(Vec<AudioCue>),
}

/// The window's handle on the audio thread. Dropping it stops the thread.
pub struct LiveAudio {
    sender: Sender<Message>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl LiveAudio {
    /// Starts the audio thread. It reports a problem (no device, missing sound data) on stderr
    /// and then idles, so the game goes on without sound.
    pub fn start(streaming_assets: &Path, game_data: &Path) -> Self {
        let (sender, receiver) = channel();
        let streaming_assets = streaming_assets.to_owned();
        let game_data = game_data.to_owned();
        let thread = std::thread::Builder::new()
            .name("tantalus-audio".to_owned())
            .spawn(move || {
                if let Err(error) = run(&streaming_assets, &game_data, &receiver) {
                    eprintln!("audio off: {error}");
                    // Keep draining so the window's sends stay cheap.
                    while receiver.recv().is_ok() {}
                }
            })
            .ok();
        Self { sender, thread }
    }

    pub fn cues(&self, cues: Vec<AudioCue>) {
        if !cues.is_empty() {
            let _ = self.sender.send(Message::Cues(cues));
        }
    }
}

impl Drop for LiveAudio {
    fn drop(&mut self) {
        // Closing the channel ends the thread's loop.
        let (closed, _) = channel();
        drop(std::mem::replace(&mut self.sender, closed));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(windows)]
fn run(
    streaming_assets: &Path,
    game_data: &Path,
    receiver: &Receiver<Message>,
) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use tantalus_tools::play_audio::AudioDirector;

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no audio output device")?;
    let (channels, rate) = pick_config(&device)?;
    let mut bank = SoundBank::open(streaming_assets, game_data)?;
    // PS1 renders from the owner's discs (`tantalus-tools psx-audio`, music and spell/ability
    // effect sounds) replace the Steam clips they cover; without them, or with `TANTALUS_NO_PSX_AUDIO` set, the Steam audio plays.
    if std::env::var_os("TANTALUS_NO_PSX_AUDIO").is_none() {
        let dir = tantalus_tools::psx_audio::default_dir();
        let found = bank.use_psx_audio(&dir);
        if found > 0 {
            eprintln!(
                "audio: {found} PS1 music and effect renders from {}",
                dir.display()
            );
        }
    }
    let mut director = AudioDirector::new(bank, rate)?;

    let ring: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
    let config = cpal::StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(rate),
        buffer_size: cpal::BufferSize::Default,
    };
    let source = Arc::clone(&ring);
    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _| {
                let mut ring = source.lock().expect("audio ring");
                for frame in data.chunks_mut(usize::from(channels)) {
                    // The director renders stereo; other layouts get it on the first two channels.
                    let left = ring.pop_front().unwrap_or(0.0);
                    let right = ring.pop_front().unwrap_or(0.0);
                    for (index, out) in frame.iter_mut().enumerate() {
                        *out = match index {
                            0 => left,
                            1 => right,
                            _ => 0.0,
                        };
                    }
                    if channels == 1 {
                        frame[0] = (left + right) * 0.5;
                    }
                }
            },
            |error| eprintln!("audio stream: {error}"),
            None,
        )
        .map_err(|e| format!("cannot open the audio stream: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("cannot start the audio stream: {e}"))?;

    // About 100 ms ahead of the card: new cues are heard within that, and a slow clip load
    // costs a gap instead of a stall.
    let target = (rate as usize / 10) * 2;
    loop {
        loop {
            match receiver.try_recv() {
                Ok(Message::Cues(cues)) => cues.iter().for_each(|cue| director.apply(cue)),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        let queued = ring.lock().map_err(|e| e.to_string())?.len();
        if queued < target {
            let samples = director.render(1);
            ring.lock().map_err(|e| e.to_string())?.extend(samples);
        } else {
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
    }
}

/// An f32 output config at a rate that is a whole number of frames per 1/60 s tick.
#[cfg(windows)]
fn pick_config(device: &cpal::Device) -> Result<(u16, u32), String> {
    use cpal::traits::DeviceTrait;
    let default = device
        .default_output_config()
        .map_err(|e| format!("no output config: {e}"))?;
    let channels = default.channels().max(1);
    let mut rates = vec![default.sample_rate().0, 48_000, 44_100];
    rates.retain(|rate| rate % 60 == 0);
    let supported: Vec<_> = device
        .supported_output_configs()
        .map_err(|e| format!("no output configs: {e}"))?
        .filter(|range| range.sample_format() == cpal::SampleFormat::F32)
        .collect();
    for rate in rates {
        if let Some(range) = supported.iter().find(|range| {
            range.channels() == channels
                && range.min_sample_rate().0 <= rate
                && rate <= range.max_sample_rate().0
        }) {
            return Ok((range.channels(), rate));
        }
    }
    Err("no f32 output at 44.1 or 48 kHz".to_owned())
}

#[cfg(not(windows))]
fn run(
    _streaming_assets: &Path,
    _game_data: &Path,
    _receiver: &Receiver<Message>,
) -> Result<(), String> {
    Err("sound output is only built for Windows".to_owned())
}
