//! Cutscene movies in the play window.
//!
//! The PC game keeps each movie as an Ogg Theora file (`StreamingAssets/ma/FMVnnn.bytes`, video
//! only: the sound is a separate resource, started by [`tantalus_tools::play_audio::AudioCue::Movie`]).
//! A worker thread decodes frames with the pure-Rust decoder in `tantalus_assets::pc::theora`
//! (faster than the movies' 15 frames a second) and the window shows the one that is due.

use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::Instant;

use tantalus_assets::pc::theora::decode_movie;

use crate::battle_raster::Frame;
use crate::battle_view::BattleWindow;

/// Frames a movie plays per second (all 60 movies are 15 fps).
pub const MOVIE_FPS: f64 = 15.0;

/// A decoded movie frame as a window frame.
fn to_frame(width: u32, height: u32, rgba: Vec<u8>) -> Frame {
    Frame::from_rgba(width, height, rgba)
}

/// Starts decoding `path` on a thread; frames arrive in order, a few ahead of need.
fn decode_thread(path: &Path) -> Result<Receiver<Frame>, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let (sender, receiver) = sync_channel(6);
    std::thread::Builder::new()
        .name("tantalus-movie".to_owned())
        .spawn(move || {
            let result = decode_movie(std::io::BufReader::new(file), None, |_, frame| {
                let rgba = frame.to_rgba();
                // The window closed the movie (skip): stop decoding.
                sender
                    .send(to_frame(frame.width, frame.height, rgba))
                    .map_err(|_| tantalus_assets::ImportError::invalid("movie", "closed"))
            });
            if let Err(error) = result {
                eprintln!("movie decode ended: {error}");
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(receiver)
}

/// One movie being played.
pub struct MoviePlayer {
    window: Option<BattleWindow>,
    frames: std::sync::Mutex<Receiver<Frame>>,
    started: Instant,
    /// Index of the frame on screen, if any.
    shown: i64,
    waiting: Option<Frame>,
    finished: bool,
}

impl MoviePlayer {
    pub fn start(path: &Path) -> Result<Self, String> {
        Ok(Self {
            window: None,
            frames: std::sync::Mutex::new(decode_thread(path)?),
            started: Instant::now(),
            shown: -1,
            waiting: None,
            finished: false,
        })
    }

    /// Advances to the frame due now. Returns true when the movie is over.
    pub fn update(&mut self, world: &mut bevy::ecs::world::World) -> bool {
        let due = (self.started.elapsed().as_secs_f64() * MOVIE_FPS) as i64;
        let mut latest = None;
        // Show the newest frame that is due (skipping those a slow window missed).
        while self.shown < due && !self.finished {
            let next = match self.waiting.take() {
                Some(frame) => Ok(frame),
                None => self.frames.lock().expect("movie frames").try_recv(),
            };
            match next {
                Ok(frame) => {
                    self.shown += 1;
                    latest = Some(frame);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => self.finished = true,
            }
        }
        if let Some(frame) = latest {
            match &self.window {
                Some(window) => window.show(world, frame),
                None => self.window = Some(BattleWindow::open(world, &frame)),
            }
        }
        self.finished
    }

    pub fn close(self, world: &mut bevy::ecs::world::World) {
        if let Some(window) = self.window {
            window.close(world);
        }
    }
}

/// `capture-movie --movie <file> --frame N --output <png>`: decodes one frame to a PNG.
pub fn run_capture(args: &[std::ffi::OsString]) -> Result<(), String> {
    let mut movie = None;
    let mut frame = 0_u64;
    let mut output = None;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("option needs a value")?;
        match flag.to_str() {
            Some("--movie") => movie = Some(std::path::PathBuf::from(value)),
            Some("--frame") => {
                frame = value
                    .to_str()
                    .and_then(|text| text.parse().ok())
                    .ok_or("--frame needs a number")?;
            }
            Some("--output") => output = Some(std::path::PathBuf::from(value)),
            _ => return Err("capture-movie takes --movie, --frame and --output".to_owned()),
        }
    }
    let (Some(movie), Some(output)) = (movie, output) else {
        return Err("capture-movie requires --movie and --output".to_owned());
    };
    let file = std::fs::File::open(&movie)
        .map_err(|error| format!("cannot open {}: {error}", movie.display()))?;
    let mut picture = None;
    let _ = decode_movie(
        std::io::BufReader::new(file),
        Some(frame + 1),
        |index, f| {
            if index == frame {
                picture = Some(to_frame(f.width, f.height, f.to_rgba()));
            }
            Ok(())
        },
    );
    let picture = picture.ok_or("the movie has fewer frames")?;
    crate::battle_view::save_png(&picture, &output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_wraps_rgba() {
        let frame = to_frame(2, 1, vec![0; 8]);
        assert_eq!((frame.width, frame.height, frame.rgba.len()), (2, 1, 8));
    }
}
