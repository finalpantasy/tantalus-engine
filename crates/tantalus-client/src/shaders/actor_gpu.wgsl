// A field actor drawn from vertex buffers, after PSX/FieldMapActor: the
// texel times the vertex color (which carries 2 * _Color), cut out where
// texel alpha times vertex alpha is under 0.5. The values are the sRGB
// encoded bytes the reference multiplies, converted to linear on output.
#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var atlas: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var atlas_sampler: sampler;

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(atlas, atlas_sampler, mesh.uv);
    if texel.a * mesh.color.a < 0.5 {
        discard;
    }
    let lit = min(texel.rgb * max(mesh.color.rgb, vec3<f32>(0.0)), vec3<f32>(1.0));
    return vec4<f32>(to_linear(lit), 1.0);
}
