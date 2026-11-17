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

/// Render layer of the black camera behind a movie, which has nothing in it.
const BACKDROP_LAYER: usize = 30;

/// A black camera over the whole window, under the movie's: the movie keeps
/// its own aspect (pillarboxed, never stretched) and the bars are black, not
/// the field the movie covers.
fn open_backdrop(world: &mut bevy::ecs::world::World) -> bevy::ecs::entity::Entity {
    use bevy::camera::RenderTarget;
    use bevy::camera::visibility::RenderLayers;
    use bevy::prelude::*;
    let mut targets = world.query_filtered::<&RenderTarget, With<Camera2d>>();
    let target = targets.iter(world).next().cloned();
    let camera = world
        .spawn((
            Camera2d,
            Camera {
                order: 9,
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            RenderLayers::layer(BACKDROP_LAYER),
        ))
        .id();
    if let Some(target) = target {
        world.entity_mut(camera).insert(target);
    }
    camera
}

/// `frame` at its own aspect, centered in a black `width` x `height` window
/// (what the live window shows for a movie).
pub fn pillarboxed(frame: &Frame, width: u32, height: u32) -> Frame {
    let aspect = frame.width as f32 / frame.height.max(1) as f32;
    let (x, y, w, h) = crate::widescreen::fit_rect(width, height, aspect);
    let mut rgba = vec![0_u8; (width * height * 4) as usize];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    for row in 0..h {
        let from_row = (row * frame.height / h.max(1)).min(frame.height - 1);
        for column in 0..w {
            let from = ((from_row * frame.width + column * frame.width / w.max(1)) * 4) as usize;
            let to = (((y + row) * width + x + column) * 4) as usize;
            rgba[to..to + 4].copy_from_slice(&frame.rgba[from..from + 4]);
        }
    }
    Frame::from_rgba(width, height, rgba)
}

/// One movie being played.
pub struct MoviePlayer {
    backdrop: Option<bevy::ecs::entity::Entity>,
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
            backdrop: None,
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
                None => {
                    self.backdrop = Some(open_backdrop(world));
                    self.window = Some(BattleWindow::open(world, &frame));
                }
            }
        }
        self.finished
    }

    pub fn close(self, world: &mut bevy::ecs::world::World) {
        if let Some(backdrop) = self.backdrop
            && let Ok(entity) = world.get_entity_mut(backdrop)
        {
            entity.despawn();
        }
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
    let mut size = None;
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
            Some("--size") => {
                size = value
                    .to_str()
                    .and_then(|text| text.split_once('x'))
                    .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
                    .filter(|&(w, h)| w > 0 && h > 0);
                if size.is_none() {
                    return Err("--size must be WxH".to_owned());
                }
            }
            _ => {
                return Err("capture-movie takes --movie, --frame, --output and --size".to_owned());
            }
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
    let mut picture = picture.ok_or("the movie has fewer frames")?;
    // `--size WxH`: as the play window shows it, pillarboxed at its own aspect.
    if let Some((width, height)) = size {
        picture = pillarboxed(&picture, width, height);
    }
    crate::battle_view::save_png(&picture, &output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movies_are_pillarboxed_not_stretched() {
        // A 4x3 picture in a 16x9 window: 12x9 in the middle, 2 black columns each side.
        let picture = Frame::from_rgba(4, 3, vec![200; 4 * 3 * 4]);
        let shown = pillarboxed(&picture, 16, 9);
        let pixel = |x: usize, y: usize| shown.rgba[(y * 16 + x) * 4];
        assert_eq!(
            (pixel(1, 4), pixel(2, 4), pixel(13, 4), pixel(14, 4)),
            (0, 200, 200, 0)
        );
    }

    #[test]
    fn frame_wraps_rgba() {
        let frame = to_frame(2, 1, vec![0; 8]);
        assert_eq!((frame.width, frame.height, frame.rgba.len()), (2, 1, 8));
    }
}
