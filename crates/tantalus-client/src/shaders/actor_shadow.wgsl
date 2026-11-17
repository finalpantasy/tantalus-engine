// A field actor's shadow quad, after PSX/FieldMapActorShadow: `r0 = texel *
// vertexColor * _Color`, killed when r, g and b times `r0.a` are all under
// 0.005, else `2 * r0.a * r0.rgb` is subtracted (`BlendOp RevSub`, set in
// `ActorShadowMaterial::specialize`). The vertex color carries the shadow
// strength in r and the quad alpha in a. Mirrors `field_light::shadow_texel`.
#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var plate: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var plate_sampler: sampler;

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(plate, plate_sampler, mesh.uv);
    let alpha = texel.a * mesh.color.a;
    let rgb = texel.rgb * mesh.color.r;
    if all(rgb * alpha < vec3<f32>(0.005)) {
        discard;
    }
    let out = min(2.0 * alpha * rgb, vec3<f32>(1.0));
    return vec4<f32>(to_linear(out), 1.0);
}
