// Field SPS effects after Memoria's `PSX/FieldSPS_Abr_0`..`_3` and `_None` fragment programs
// (Shaders/PSX/FieldSPS_*.txt). The texture is read as raw 8-bit values and the product with the
// vertex color is taken in that space, as the reference does (no color-space conversion); the
// result is converted to linear for the sRGB target. The blend state set in `SpsMaterial::specialize`
// does the rest: mode 1 and 3 add, mode 2 subtracts, mode 0 and 4 draw over.
#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct SpsParams {
    // 0 to 3 are the PSX blend modes; 4 is `_None`.
    mode: u32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: SpsParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var page: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var page_sampler: sampler;

fn to_linear(value: vec3<f32>) -> vec3<f32> {
    let low = value / 12.92;
    let high = pow((value + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, value <= vec3<f32>(0.04045));
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(page, page_sampler, mesh.uv);
    let product = texel.rgb * mesh.color.rgb;
    switch params.mode {
        case 1u: {
            // `texkill` unless one channel of texel * color * alpha reaches 0.035.
            let lit = product * texel.a;
            if lit.r < 0.035 && lit.g < 0.035 && lit.b < 0.035 {
                discard;
            }
            return vec4<f32>(to_linear(lit), 1.0);
        }
        case 3u: {
            // The PSX quarter weight: the shader scales the color by 0.25 before the add.
            return vec4<f32>(to_linear(product * 0.25), 1.0);
        }
        default: {
            if texel.a * mesh.color.a < 0.1 {
                discard;
            }
            return vec4<f32>(to_linear(product * texel.a), texel.a);
        }
    }
}
