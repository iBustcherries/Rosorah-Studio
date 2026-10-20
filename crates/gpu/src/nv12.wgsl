// A finished canvas (gamma-encoded RGBA) to NV12 in BT.709 video range, the picture format
// encoders and cameras take. Each invocation makes a 4x2 block of pixels: two words of luma and
// one word of interleaved chroma. Outside the fitted area the picture is black.
struct Params {
    size: vec2<u32>,
    origin: vec2<u32>,
    fit: vec2<u32>,
    source: vec2<u32>,
};
@group(0) @binding(0) var canvas: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var<storage, read_write> picture: array<u32>;
@group(0) @binding(3) var<uniform> p: Params;

fn colour(x: u32, y: u32) -> vec3<f32> {
    if (x < p.origin.x || y < p.origin.y || x >= p.origin.x + p.fit.x || y >= p.origin.y + p.fit.y) {
        return vec3<f32>(0.0);
    }
    // Pixel centres; at the canvas's own size this lands exactly on each pixel.
    let at = (vec2<f32>(vec2<u32>(x, y) - p.origin) + 0.5) / vec2<f32>(p.fit);
    return textureSampleLevel(canvas, linear_sampler, at, 0.0).rgb;
}
fn byte(v: f32) -> u32 {
    return u32(clamp(round(v), 0.0, 255.0));
}
fn luma(c: vec3<f32>) -> u32 {
    return byte(16.0 + 219.0 * dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = id.x * 4u;
    let y = id.y * 2u;
    if (x >= p.size.x || y >= p.size.y) {
        return;
    }
    var rows = array<u32, 2>(0u, 0u);
    var chroma = 0u;
    for (var pair = 0u; pair < 2u; pair++) {
        var sum = vec3<f32>(0.0);
        for (var dx = 0u; dx < 2u; dx++) {
            let column = pair * 2u + dx;
            for (var dy = 0u; dy < 2u; dy++) {
                let c = colour(x + column, y + dy);
                sum += c;
                rows[dy] |= luma(c) << (column * 8u);
            }
        }
        // Chroma from the average of the 2x2 block.
        let c = sum * 0.25;
        let cb = 128.0 + 224.0 * dot(c, vec3<f32>(-0.1146, -0.3854, 0.5));
        let cr = 128.0 + 224.0 * dot(c, vec3<f32>(0.5, -0.4542, -0.0458));
        chroma |= (byte(cb) | (byte(cr) << 8u)) << (pair * 16u);
    }
    let words = p.size.x / 4u;
    picture[y * words + id.x] = rows[0];
    picture[(y + 1u) * words + id.x] = rows[1];
    picture[p.size.x * p.size.y / 4u + (y / 2u) * words + id.x] = chroma;
}
