use tantalus_contracts::{FieldLayerPresentation, PresentationSnapshot};

pub struct LegacyCompositor;

impl LegacyCompositor {
    pub fn ordered_layers(snapshot: &PresentationSnapshot) -> Vec<FieldLayerPresentation> {
        let mut layers = snapshot.layers.clone();
        layers.sort_by_key(|layer| (layer.position.z, layer.foreground));
        layers
    }
}
