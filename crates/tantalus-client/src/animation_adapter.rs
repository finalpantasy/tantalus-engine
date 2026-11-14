use bevy::prelude::Component;
use tantalus_contracts::{ContentId, Tick};

#[derive(Debug, Clone, PartialEq, Eq, Component)]
pub struct PresentationAnimation {
    pub clip: ContentId,
    pub started_at: Tick,
}

pub struct AnimationAdapter;

impl AnimationAdapter {
    pub fn map(clip: Option<ContentId>, tick: Tick) -> Option<PresentationAnimation> {
        clip.map(|clip| PresentationAnimation {
            clip,
            started_at: tick,
        })
    }
}
