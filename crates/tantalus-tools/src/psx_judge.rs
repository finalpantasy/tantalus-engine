//! `psx-audio sfx-judge` and `psx-audio apply-choices`: which of the two clips of a sound, the
//! PS1 render or the Steam clip, the client plays.
//!
//! The PS1 render wins by default. The Steam clip wins only where a measurement shows the render
//! is broken: near-silent, cut short, clipped, or with a loop seam that clicks. A pair whose
//! measurements are borderline or disagree is "unsure" (the PS1 render plays until someone
//! listens and decides). The picks go to [`PICKS`] next to the manifest; a listener's choices
//! (`choices.json` from the listening page) go to [`OVERRIDES`] and win over the picks. Both files
//! are read when the clips are opened ([`crate::psx_audio::PsxClips::open`]), so a Steam pick
//! simply leaves the Steam clip in place.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tantalus_legacy::sound::PcmClip;

use crate::psx_audio::{ManifestEntry, PsxClips, band_profile, mono, pearson};
use crate::sound_bank::SoundBank;

/// The rule-based picks, next to the manifest.
pub const PICKS: &str = "psx-audio-picks.json";
/// A listener's choices, next to the manifest; they win over [`PICKS`].
pub const OVERRIDES: &str = "psx-audio-overrides.json";

/// Which clip plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pick {
    Ps1,
    Steam,
    /// The measurements cannot settle it; the PS1 render plays until a listener decides.
    Unsure,
    /// A listener heard no reason to prefer either; the PS1 render plays.
    Either,
}

impl Pick {
    /// True when the Steam clip plays instead of the render.
    pub fn uses_steam(self) -> bool {
        self == Pick::Steam
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickEntry {
    pub pick: Pick,
    /// Why, in plain words.
    pub why: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PickFile {
    pub version: u32,
    pub picks: BTreeMap<String, PickEntry>,
}

/// The listening page's download, and the overrides file `apply-choices` writes.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Choices {
    #[serde(default)]
    pub version: u32,
    pub choices: BTreeMap<String, Pick>,
}

/// The pick for each resource once the overrides are laid over the rule-based picks.
pub fn read_picks(dir: &Path) -> BTreeMap<String, Pick> {
    let mut out = BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(dir.join(PICKS))
        && let Ok(file) = serde_json::from_str::<PickFile>(&text)
    {
        out.extend(file.picks.into_iter().map(|(r, e)| (r, e.pick)));
    }
    if let Ok(text) = std::fs::read_to_string(dir.join(OVERRIDES))
        && let Ok(file) = serde_json::from_str::<Choices>(&text)
    {
        out.extend(file.choices);
    }
    out
}

/// What one clip measures.
#[derive(Debug, Clone, Copy, Default)]
pub struct Measure {
    pub seconds: f64,
    /// Until the sound falls 40 dB under its peak.
    pub sounding: f64,
    pub peak_db: f64,
    /// RMS over the sounding part, dBFS.
    pub rms_db: f64,
    /// Frequency under which 99% of the energy lies (Hz): the usable bandwidth.
    pub rolloff_hz: f64,
    /// The quietest tenth of 10 ms frames inside the sounding part, dBFS (-120 = digital silence).
    pub floor_db: f64,
    /// Fraction of samples in flat runs at full scale.
    pub clipped: f64,
    /// Loop length in seconds, when the clip loops.
    pub loop_seconds: Option<f64>,
    /// The step across the loop seam against the clip's typical sample step (1 = seamless).
    pub seam: f64,
}

fn db(x: f64) -> f64 {
    if x <= 1e-6 { -120.0 } else { 20.0 * x.log10() }
}

/// Measures `clip`, whose loop (when it has one) runs from frame `loop_start` to `loop_end`
/// inclusive.
pub fn measure(clip: &PcmClip) -> Measure {
    let rate = clip.sample_rate.max(1);
    let x = mono(clip);
    let n = x.len();
    let mut m = Measure {
        seconds: n as f64 / f64::from(rate),
        ..Measure::default()
    };
    if n == 0 {
        m.peak_db = -120.0;
        m.rms_db = -120.0;
        m.floor_db = -120.0;
        return m;
    }
    let peak = x.iter().fold(0f32, |a, v| a.max(v.abs()));
    m.peak_db = db(f64::from(peak));
    let first = x.iter().position(|v| v.abs() >= peak * 0.01).unwrap_or(0);
    let last = x.iter().rposition(|v| v.abs() >= peak * 0.01).unwrap_or(0);
    m.sounding = (last + 1) as f64 / f64::from(rate);
    let body = &x[first..=last.max(first)];
    let rms = (body.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / body.len() as f64).sqrt();
    m.rms_db = db(rms);
    // Noise floor: the quietest frames while the sound is on.
    let frame = (rate / 100) as usize;
    let mut levels: Vec<f64> = body
        .chunks(frame.max(1))
        .filter(|c| c.len() == frame)
        .map(|c| (c.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / c.len() as f64).sqrt())
        .collect();
    levels.sort_by(f64::total_cmp);
    m.floor_db = levels.get(levels.len() / 10).map_or(-120.0, |l| db(*l));
    // Clipping: runs of three or more samples (any channel) at full scale.
    let mut clipped = 0usize;
    let ch = usize::from(clip.channels.max(1));
    for c in 0..ch {
        let mut run = 0;
        for f in 0..clip.frames() {
            if i32::from(clip.samples[f * ch + c]).abs() >= 32_700 {
                run += 1;
                if run == 3 {
                    clipped += 3;
                } else if run > 3 {
                    clipped += 1;
                }
            } else {
                run = 0;
            }
        }
    }
    m.clipped = clipped as f64 / clip.samples.len().max(1) as f64;
    m.rolloff_hz = rolloff(body, rate, 0.99);
    if clip.loop_end > clip.loop_start && (clip.loop_end as usize) < n {
        let (s, e) = (clip.loop_start as usize, clip.loop_end as usize);
        m.loop_seconds = Some((e + 1 - s) as f64 / f64::from(rate));
        // The wrap plays x[e] then x[s]; compare that step with the typical step inside the loop.
        let mut steps: Vec<f64> = (s + 1..=e)
            .map(|i| f64::from((x[i] - x[i - 1]).abs()))
            .collect();
        steps.sort_by(f64::total_cmp);
        let typical = steps.get(steps.len() * 9 / 10).copied().unwrap_or(0.0);
        let jump = f64::from((x[s] - x[e]).abs());
        m.seam = jump / typical.max(1e-4);
    }
    m
}

/// The frequency under which `fraction` of the energy of `x` lies (2048-point frames, the
/// first 6 seconds).
fn rolloff(x: &[f32], rate: u32, fraction: f64) -> f64 {
    const N: usize = 2048;
    let mut power = vec![0f64; N / 2];
    let limit = x.len().min(rate as usize * 6);
    let mut frame = 0;
    while frame < limit {
        let mut re: Vec<f64> = (0..N)
            .map(|i| {
                let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / N as f64).cos();
                f64::from(x.get(frame + i).copied().unwrap_or(0.0)) * w
            })
            .collect();
        let mut im = vec![0f64; N];
        crate::psx_audio::fft(&mut re, &mut im);
        for k in 1..N / 2 {
            power[k] += re[k] * re[k] + im[k] * im[k];
        }
        frame += N;
    }
    let total: f64 = power.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    let mut acc = 0.0;
    for (k, p) in power.iter().enumerate() {
        acc += p;
        if acc >= total * fraction {
            return k as f64 * f64::from(rate) / N as f64;
        }
    }
    f64::from(rate) / 2.0
}

/// One judged pair.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub pick: Pick,
    /// The rule that decided, in plain words.
    pub why: String,
    /// How the two differ, in plain words.
    pub summary: String,
    /// Band-spectrum correlation of the two clips (1 = same shape).
    pub likeness: f64,
}

/// Seam steps above this many typical steps click.
const SEAM_BAD: f64 = 8.0;
const SEAM_DOUBT: f64 = 3.0;

/// The rule. `p` is the PS1 render, `s` the Steam clip, `likeness` their band-spectrum
/// correlation.
pub fn judge(p: &Measure, s: &Measure, likeness: f64) -> Verdict {
    let mut steam_reasons: Vec<String> = Vec::new();
    let mut doubts: Vec<String> = Vec::new();
    let level = p.rms_db - s.rms_db;
    // Near-silent.
    if p.peak_db < -40.0 && s.peak_db > -30.0 {
        steam_reasons.push(format!("PS1 is near-silent (peak {:.0} dB)", p.peak_db));
    } else if level < -30.0 {
        steam_reasons.push(format!("PS1 is {:.0} dB quieter", -level));
    } else if level < -15.0 {
        doubts.push(format!("PS1 is {:.0} dB quieter", -level));
    }
    // Cut short (a looping clip plays on, so its length says nothing).
    let ratio = p.sounding / s.sounding.max(0.01);
    let one_shots = p.loop_seconds.is_none() && s.loop_seconds.is_none();
    if !one_shots {
        // Nothing to compare.
    } else if s.sounding >= 0.3 && ratio < 0.4 {
        steam_reasons.push(format!(
            "PS1 stops at {:.2} s, Steam at {:.2} s",
            p.sounding, s.sounding
        ));
    } else if s.sounding >= 0.3 && ratio < 0.6 {
        doubts.push(format!(
            "PS1 stops at {:.2} s, Steam at {:.2} s",
            p.sounding, s.sounding
        ));
    } else if ratio > 3.0 && p.sounding > s.sounding + 1.0 {
        // Longer by a reverb tail is expected; three times as long may be a held note.
        doubts.push(format!(
            "PS1 rings {:.2} s, Steam {:.2} s",
            p.sounding, s.sounding
        ));
    }
    // Clipping.
    if p.clipped > 0.005 && s.clipped < 0.0005 {
        steam_reasons.push(format!("PS1 clips ({:.1}% of samples)", p.clipped * 100.0));
    } else if p.clipped > 0.0005 && p.clipped > s.clipped * 2.0 {
        doubts.push(format!("PS1 clips a little ({:.2}%)", p.clipped * 100.0));
    }
    // Loops.
    match (p.loop_seconds, s.loop_seconds) {
        (Some(_), _) if p.seam > SEAM_BAD && s.seam <= SEAM_DOUBT => {
            steam_reasons.push(format!("PS1 loop seam clicks ({:.0}x step)", p.seam));
        }
        (Some(_), _) if p.seam > SEAM_DOUBT => {
            doubts.push(format!("PS1 loop seam may click ({:.0}x step)", p.seam));
        }
        (Some(pl), None) => doubts.push(format!(
            "PS1 loops every {pl:.2} s, Steam plays once ({:.2} s)",
            s.seconds
        )),
        // A different loop length alone is no fault: the PS1 loop closes when every voice's own
        // loop does, which can take many of Steam's periods.
        _ => {}
    }
    if likeness < 0.2 && s.peak_db > -40.0 && p.peak_db > -40.0 {
        doubts.push(format!(
            "the two spectra are unlike (correlation {likeness:.2}), maybe not the same sound"
        ));
    }
    // A Steam clip with its own fault does not win outright.
    let steam_faults = s.clipped > 0.005 || (s.loop_seconds.is_some() && s.seam > SEAM_BAD);
    let (pick, why) = if !steam_reasons.is_empty() && !steam_faults && doubts.is_empty() {
        (Pick::Steam, steam_reasons.join("; "))
    } else if !steam_reasons.is_empty() || !doubts.is_empty() {
        let mut all = steam_reasons;
        all.extend(doubts);
        if steam_faults {
            all.push("Steam has its own fault".to_owned());
        }
        (Pick::Unsure, all.join("; "))
    } else {
        (Pick::Ps1, "no fault found in the PS1 render".to_owned())
    };
    Verdict {
        pick,
        why,
        summary: describe(p, s),
        likeness,
    }
}

/// How the two clips differ, in plain words.
pub fn describe(p: &Measure, s: &Measure) -> String {
    let mut out: Vec<String> = Vec::new();
    let bright = p.rolloff_hz / s.rolloff_hz.max(1.0);
    if bright > 1.25 {
        out.push(format!(
            "PS1 is brighter ({:.1} kHz vs {:.1} kHz)",
            p.rolloff_hz / 1000.0,
            s.rolloff_hz / 1000.0
        ));
    } else if bright < 0.8 {
        out.push(format!(
            "Steam is brighter ({:.1} kHz vs {:.1} kHz)",
            s.rolloff_hz / 1000.0,
            p.rolloff_hz / 1000.0
        ));
    } else {
        out.push("same brightness".to_owned());
    }
    let level = p.rms_db - s.rms_db;
    if level > 3.0 {
        out.push(format!("Steam is quieter by {level:.0} dB"));
    } else if level < -3.0 {
        out.push(format!("PS1 is quieter by {:.0} dB", -level));
    } else {
        out.push("same loudness".to_owned());
    }
    let d = p.sounding - s.sounding;
    if d.abs() > 0.15 && d.abs() > 0.2 * s.sounding {
        out.push(format!(
            "PS1 lasts {:.2} s, Steam {:.2} s",
            p.sounding, s.sounding
        ));
    } else {
        out.push(format!("both last about {:.2} s", s.sounding));
    }
    if s.floor_db > p.floor_db + 10.0 && s.floor_db > -80.0 {
        out.push(format!(
            "Steam has more background noise ({:.0} dB vs {:.0} dB)",
            s.floor_db, p.floor_db
        ));
    } else if p.floor_db > s.floor_db + 10.0 && p.floor_db > -80.0 {
        out.push(format!(
            "PS1 has more background noise ({:.0} dB vs {:.0} dB)",
            p.floor_db, s.floor_db
        ));
    }
    match (p.loop_seconds, s.loop_seconds) {
        (Some(pl), Some(sl)) => out.push(format!("both loop ({pl:.2} s and {sl:.2} s)")),
        (Some(pl), None) => out.push(format!("only PS1 loops ({pl:.2} s)")),
        (None, Some(sl)) => out.push(format!("only Steam loops ({sl:.2} s)")),
        (None, None) => {}
    }
    if p.clipped > 0.0005 {
        out.push(format!("PS1 clips {:.2}%", p.clipped * 100.0));
    }
    if s.clipped > 0.0005 {
        out.push(format!("Steam clips {:.2}%", s.clipped * 100.0));
    }
    out.join(", ")
}

fn empty_clip() -> PcmClip {
    PcmClip {
        sample_rate: 44_100,
        channels: 2,
        samples: Vec::new(),
        loop_start: 0,
        loop_end: 0,
        loop_start_alternate: 0,
        loop_end_alternate: 0,
    }
}

/// One row of the judgement, for the picks file and the page.
pub struct Row {
    pub entry: ManifestEntry,
    pub verdict: Verdict,
    pub ps1: PcmClip,
    pub steam: PcmClip,
}

/// Judges every sound effect render in `dir` against the Steam clip it replaces and writes
/// [`PICKS`]. Returns the rows (clips included) for the page.
pub fn judge_all(dir: &Path, steam: &mut SoundBank, keep_clips: bool) -> Result<Vec<Row>, String> {
    let clips = PsxClips::open_all(dir).ok_or("no psx-audio.json in the output folder")?;
    let mut rows = Vec::new();
    let mut file = PickFile {
        version: 1,
        picks: BTreeMap::new(),
    };
    for entry in clips.entries() {
        if !entry.kind.starts_with("sfx") {
            continue;
        }
        let (Ok(s_clip), Ok(Some(p_clip))) = (
            steam.load_clip(&entry.resource),
            clips.load(&entry.resource),
        ) else {
            continue;
        };
        let (pm, sm) = (measure(&p_clip), measure(&s_clip));
        let likeness = pearson(
            &band_profile(&mono(&p_clip), p_clip.sample_rate),
            &band_profile(&mono(&s_clip), s_clip.sample_rate),
        );
        let verdict = judge(&pm, &sm, likeness);
        file.picks.insert(
            entry.resource.clone(),
            PickEntry {
                pick: verdict.pick,
                why: verdict.why.clone(),
            },
        );
        let (ps1, steam) = if keep_clips {
            (p_clip, s_clip)
        } else {
            (empty_clip(), empty_clip())
        };
        rows.push(Row {
            entry: entry.clone(),
            verdict,
            ps1,
            steam,
        });
    }
    let path = dir.join(PICKS);
    let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    let count = |k: Pick| rows.iter().filter(|r| r.verdict.pick == k).count();
    println!(
        "judged {} sound pairs: PS1 kept {}, Steam kept {}, unsure {} (written to {})",
        rows.len(),
        count(Pick::Ps1),
        count(Pick::Steam),
        count(Pick::Unsure),
        path.display()
    );
    Ok(rows)
}

/// `apply-choices`: lays a listening page's `choices.json` over [`OVERRIDES`] in `dir`.
pub fn apply_choices(dir: &Path, choices: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(choices)
        .map_err(|e| format!("cannot read {}: {e}", choices.display()))?;
    let new: Choices =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", choices.display()))?;
    let path = dir.join(OVERRIDES);
    let mut all: Choices = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    all.version = 1;
    let mut steam = 0;
    for (resource, pick) in new.choices {
        if pick == Pick::Unsure {
            continue;
        }
        steam += usize::from(pick.uses_steam());
        all.choices.insert(resource, pick);
    }
    let text = serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    println!(
        "{} overrides in {} ({steam} of the new ones use the Steam clip)",
        all.choices.len(),
        path.display()
    );
    Ok(())
}

/// The listening page: every unsure pair and up to `sample` pairs of each decided kind, with
/// the clips as WAV files under `page/clips`.
pub fn write_page(page: &Path, rows: &[Row], sample: usize) -> Result<usize, String> {
    let clips_dir = page.join("clips");
    std::fs::create_dir_all(&clips_dir).map_err(|e| format!("{}: {e}", clips_dir.display()))?;
    let spread = |kind: Pick| -> Vec<&Row> {
        let all: Vec<&Row> = rows.iter().filter(|r| r.verdict.pick == kind).collect();
        if kind == Pick::Unsure || all.len() <= sample {
            return all;
        }
        (0..sample).map(|i| all[i * all.len() / sample]).collect()
    };
    let mut chosen: Vec<&Row> = spread(Pick::Unsure);
    chosen.extend(spread(Pick::Steam));
    chosen.extend(spread(Pick::Ps1));
    let mut html = String::from(PAGE_HEAD);
    for row in &chosen {
        let e = &row.entry;
        let base = e
            .resource
            .rsplit('/')
            .next()
            .unwrap_or(&e.resource)
            .to_owned();
        let (p_name, s_name) = (format!("{base}.ps1.wav"), format!("{base}.steam.wav"));
        write_listen_wav(&clips_dir.join(&p_name), &row.ps1)?;
        write_listen_wav(&clips_dir.join(&s_name), &row.steam)?;
        let auto = match row.verdict.pick {
            Pick::Ps1 => "PS1",
            Pick::Steam => "Steam",
            _ => "unsure",
        };
        let id = escape(&e.resource);
        html.push_str(&format!(
            r#"<section data-r="{id}" data-auto="{auto}">
<h2>{sid} {name}</h2>
<p class="where">{place}</p>
<div class="players"><label>PS1 <audio controls preload="none" src="clips/{p}"></audio></label>
<label>Steam <audio controls preload="none" src="clips/{s}"></audio></label></div>
<p>{summary}.</p>
<p class="auto">Auto pick: <b>{auto}</b> ({why}).</p>
<p class="pick"><label><input type="radio" name="{id}" value="ps1">PS1</label>
<label><input type="radio" name="{id}" value="steam">Steam</label>
<label><input type="radio" name="{id}" value="either">Either</label></p>
</section>
"#,
            sid = e.sound_id,
            name = escape(&e.resource),
            place = escape(&place(e)),
            p = escape(&p_name),
            s = escape(&s_name),
            summary = escape(&row.verdict.summary),
            why = escape(&row.verdict.why),
        ));
    }
    html.push_str(PAGE_TAIL);
    let path = page.join("index.html");
    std::fs::write(&path, html).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(chosen.len())
}

/// Where the sound plays, as far as the mapping knows.
fn place(e: &ManifestEntry) -> String {
    match e.kind.as_str() {
        "sfx" => format!(
            "Battle or field effect {} (sound {} of the effect)",
            e.ps1_sequence_id / 16,
            e.ps1_sequence_id % 16
        ),
        "sfx-field" => format!("Field or menu sound (sound id {})", e.ps1_sequence_id),
        "sfx-resident" => format!(
            "Battle sound loaded for every battle (slot {})",
            e.ps1_sequence_id
        ),
        other => other.to_owned(),
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The clip as a WAV for listening: a looping clip is written as its intro and two passes of
/// its loop, so the seam is heard.
fn write_listen_wav(path: &Path, clip: &PcmClip) -> Result<(), String> {
    let ch = usize::from(clip.channels.max(1));
    let mut samples = clip.samples.clone();
    if clip.loop_end > clip.loop_start && (clip.loop_end as usize) < clip.frames() {
        samples.truncate((clip.loop_end as usize + 1) * ch);
        let body =
            clip.samples[clip.loop_start as usize * ch..(clip.loop_end as usize + 1) * ch].to_vec();
        samples.extend_from_slice(&body);
    }
    crate::psx_audio::write_wav(path, clip.sample_rate, clip.channels.max(1), &samples)
}

const PAGE_HEAD: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>PS1 or Steam: sound judge</title>
<style>
body{font-family:system-ui,sans-serif;margin:0 auto;max-width:46rem;padding:0.8rem;line-height:1.4}
section{border-top:1px solid #bbb;padding:0.6rem 0}
h2{font-size:1rem;margin:0;word-break:break-all}
.where,.auto{color:#555;font-size:0.9rem;margin:0.2rem 0}
.players label{display:block;margin:0.3rem 0}
audio{width:100%;max-width:100%}
.pick label{margin-right:1.2rem;font-size:1.05rem}
#bar{position:sticky;top:0;background:#fff;padding:0.4rem 0;border-bottom:1px solid #888}
button{font-size:1rem;padding:0.4rem 1rem}
</style></head><body>
<div id="bar"><button id="save">Save choices</button> <span id="count"></span></div>
<p>Each row is one sound: the PS1 render from your discs and the Steam clip. Pick the one that
sounds right, or Either. Unsure rows come first, then sample rows the rule decided. Save
choices downloads <code>choices.json</code>; apply it with
<code>tantalus-tools psx-audio apply-choices --choices choices.json</code>.</p>
"#;

const PAGE_TAIL: &str = r#"<script>
const rows=[...document.querySelectorAll('section')];
function count(){const n=rows.filter(r=>r.querySelector('input:checked')).length;
document.getElementById('count').textContent=n+' of '+rows.length+' chosen';}
document.addEventListener('change',count);count();
document.getElementById('save').onclick=()=>{const choices={};
for(const r of rows){const c=r.querySelector('input:checked');if(c)choices[r.dataset.r]=c.value;}
const blob=new Blob([JSON.stringify({version:1,choices},null,1)],{type:'application/json'});
const a=document.createElement('a');a.href=URL.createObjectURL(blob);a.download='choices.json';a.click();};
</script></body></html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(seconds: f64, amp: f32) -> PcmClip {
        let rate = 22_050u32;
        let n = (seconds * f64::from(rate)) as usize;
        PcmClip {
            sample_rate: rate,
            channels: 1,
            samples: (0..n)
                .map(|i| {
                    let t = i as f32 / rate as f32;
                    ((std::f32::consts::TAU * 880.0 * t).sin() * amp * 32767.0) as i16
                })
                .collect(),
            loop_start: 0,
            loop_end: 0,
            loop_start_alternate: 0,
            loop_end_alternate: 0,
        }
    }

    #[test]
    fn a_clean_render_keeps_ps1_and_a_cut_one_goes_to_steam() {
        let steam = measure(&tone(1.0, 0.5));
        let same = measure(&tone(1.0, 0.5));
        assert_eq!(judge(&same, &steam, 0.9).pick, Pick::Ps1);
        let cut = measure(&tone(0.2, 0.5));
        let v = judge(&cut, &steam, 0.9);
        assert_eq!(v.pick, Pick::Steam, "{}", v.why);
        let borderline = measure(&tone(0.55, 0.5));
        assert_eq!(judge(&borderline, &steam, 0.9).pick, Pick::Unsure);
        let silent = measure(&tone(1.0, 0.001));
        assert_eq!(judge(&silent, &steam, 0.9).pick, Pick::Steam);
    }

    #[test]
    fn clipping_and_a_clicking_seam_are_measured() {
        let hot = measure(&tone(1.0, 2.0));
        assert!(hot.clipped > 0.05, "{}", hot.clipped);
        let mut looped = tone(1.0, 0.5);
        looped.loop_start = 0;
        // Ends a quarter period off the start: a jump of the full amplitude.
        looped.loop_end = 22_050 / 2 + 6;
        let m = measure(&looped);
        assert!(m.seam > SEAM_DOUBT, "seam {}", m.seam);
    }

    #[test]
    fn overrides_win_over_picks() {
        let dir = std::env::temp_dir().join(format!("psx-judge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut picks = PickFile::default();
        for (r, p) in [("a", Pick::Steam), ("b", Pick::Ps1)] {
            picks.picks.insert(
                r.to_owned(),
                PickEntry {
                    pick: p,
                    why: String::new(),
                },
            );
        }
        std::fs::write(dir.join(PICKS), serde_json::to_string(&picks).unwrap()).unwrap();
        let choices = dir.join("choices.json");
        std::fs::write(
            &choices,
            r#"{"version":1,"choices":{"a":"ps1","b":"steam"}}"#,
        )
        .unwrap();
        apply_choices(&dir, &choices).unwrap();
        let all = read_picks(&dir);
        assert_eq!(all["a"], Pick::Ps1);
        assert_eq!(all["b"], Pick::Steam);
        std::fs::remove_dir_all(&dir).ok();
    }
}
