// The battle and world-map frame on the GPU (`frame_gpu.rs`): the triangles,
// rectangles and HUD images `battle_raster::Frame` records, drawn in order.
// The target is a plain RGBA8 view of the frame image, so blending works on
// the sRGB-encoded bytes the CPU rasterizer blends.

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    // x: 1 for a box-filtered image; y: its source texels per frame pixel.
    @location(2) @interpolate(flat) mode: vec2<f32>,
}

@group(0) @binding(0) var texels: texture_2d<f32>;
@group(0) @binding(1) var texel_sampler: sampler;
// Per draw (the instance index): to clip space. The first is the identity,
// for triangles recorded in clip space.
@group(1) @binding(0) var<storage, read> matrices: array<mat4x4<f32>>;

@vertex
fn vertex(
    @builtin(instance_index) instance: u32,
    @location(0) clip: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mode: vec2<f32>,
) -> VertexOut {
    var out: VertexOut;
    out.position = matrices[instance] * clip;
    out.uv = uv;
    out.color = color;
    out.mode = mode;
    return out;
}

// `battle_hud::blit_scaled`: the premultiplied average of the source texels
// under the frame pixel, its coverage the share of the pixel they fill.
fn box_filter(uv: vec2<f32>, per_pixel: f32) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(texels));
    let at = uv * size;
    let low = max(at - vec2<f32>(per_pixel * 0.5), vec2<f32>(0.0));
    let high = min(at + vec2<f32>(per_pixel * 0.5), size);
    var sum = vec3<f32>(0.0);
    var alpha = 0.0;
    var y = low.y;
    for (var row_step = 0; row_step < 12; row_step++) {
        if y >= high.y {
            break;
        }
        let row = floor(y);
        let h = min(row + 1.0, high.y) - y;
        var x = low.x;
        for (var column_step = 0; column_step < 12; column_step++) {
            if x >= high.x {
                break;
            }
            let column = floor(x);
            let w = min(column + 1.0, high.x) - x;
            let texel = textureLoad(texels, vec2<i32>(i32(column), i32(row)), 0);
            let area = w * h;
            sum += texel.rgb * texel.a * area;
            alpha += texel.a * area;
            x = column + 1.0;
        }
        y = row + 1.0;
    }
    if alpha <= 0.0 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(sum / alpha, clamp(alpha / (per_pixel * per_pixel), 0.0, 1.0));
}

fn shade(in: VertexOut) -> vec4<f32> {
    var texel: vec4<f32>;
    if in.mode.x > 0.5 {
        texel = box_filter(in.uv, in.mode.y);
    } else {
        texel = textureSampleLevel(texels, texel_sampler, in.uv, 0.0);
    }
    return texel * in.color;
}

// `Blend::Opaque`: cut out under alpha 0.5, then replace.
@fragment
fn cutout(in: VertexOut) -> @location(0) vec4<f32> {
    let color = shade(in);
    if color.a < 0.5 {
        discard;
    }
    return vec4<f32>(color.rgb, 1.0);
}

// Screen, multiply and straight alpha: the pipeline's blend state does the rest.
@fragment
fn plain(in: VertexOut) -> @location(0) vec4<f32> {
    return shade(in);
}

// `world_weather::Weather::apply`: the sky where nothing wrote depth, the
// far ground faded into the fog.
struct Fog {
    sky: vec4<f32>,
    fog: vec4<f32>,
    // Horizon row, fog start and end distances.
    params: vec4<f32>,
}

@group(0) @binding(0) var depth: texture_depth_2d;
@group(0) @binding(1) var<uniform> fog: Fog;

@vertex
fn fog_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32(index & 1u) * 4.0 - 1.0;
    let y = f32(index >> 1u) * 4.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fog_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    // Depth holds 1 / view distance, 0 where nothing drew.
    let stored = textureLoad(depth, vec2<i32>(position.xy), 0);
    if stored <= 0.0 {
        let horizon = fog.params.x;
        var t = 1.0;
        if horizon > 0.0 {
            t = clamp((position.y - 0.5) / horizon, 0.0, 1.0);
        }
        return vec4<f32>(mix(fog.sky.rgb, fog.fog.rgb, t * t), 1.0);
    }
    let distance = 1.0 / stored;
    var k = clamp((distance - fog.params.y) / (fog.params.z - fog.params.y), 0.0, 1.0);
    if k <= 0.0 {
        discard;
    }
    k = k * k * (3.0 - 2.0 * k);
    return vec4<f32>(fog.fog.rgb, k);
}
