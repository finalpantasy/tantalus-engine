// The script fade filter (`FADEFILTER`): Memoria's shaders add or subtract
// `_FadeColor_ABR1/2` from every pixel. This quad does the same over the field; the blend
// state set in `FadeBlendMaterial::specialize` picks add or reverse-subtract.
#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct FadeBlend {
    color: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: FadeBlend;

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(material.color.rgb, 0.0);
}
