// Field background sprites with blend modes 1-3, after Memoria's
// `PSX/FieldMap_Abr_1`..`_3`: the color is premultiplied by the texel
// alpha and texels under the kill threshold are dropped. The blend state
// set in `PsxBlendMaterial::specialize` does the add or subtract.
#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct PsxBlend {
    // Atlas rectangle as (u0, v0, u1, v1), normalized.
    rect: vec4<f32>,
    // Texels with alpha below this are dropped (`texkill`).
    kill_below: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: PsxBlend;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var atlas: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var atlas_sampler: sampler;

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let uv = mix(material.rect.xy, material.rect.zw, mesh.uv);
    let texel = textureSample(atlas, atlas_sampler, uv);
    if texel.a < material.kill_below {
        discard;
    }
    return vec4<f32>(texel.rgb * texel.a, texel.a);
}
