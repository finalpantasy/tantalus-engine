//! PS1 audio from the owner's own Final Fantasy IX discs.
//!
//! Read-only disc access (`disc`), the `FF9.IMG` archive (`img`), Square's AKAO sample banks and
//! sequences (`akao`), a PS1 SPU model (`spu`), the sequencer that drives it (`sequencer`) and an
//! offline renderer (`render`). Nothing here ships or stores game audio: everything is read from
//! the user's discs at run time and written under the user's own data folder.
//!
//! References (documentation only, no code copied): psx-spx "Sound Processing Unit" for the SPU
//! (<https://psx-spx.consoledev.net/soundprocessingunitspu/>); the ff7-flat-wiki "AKAO sequence"
//! page for the sequence format (<https://ff7-mods.github.io/ff7-flat-wiki/FF9/Sound/AKAO_sequence>);
//! and VGMTrans' Akao reader (zlib license, <https://github.com/vgmtrans/vgmtrans>) for the sample
//! collection header and drum/instrument map layouts and the event semantics of version 3.2.

pub mod akao;
pub mod cluster;
pub mod disc;
pub mod img;
pub mod render;
pub mod sequencer;
pub mod sfx;
pub mod spu;
pub mod tables;
