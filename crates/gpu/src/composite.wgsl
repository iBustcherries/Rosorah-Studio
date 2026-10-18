// Rosorah's canvas compositor. Each vertex names its layer; the layer's settings live in one table
// written once per frame, so effects cost nothing for layers that don't use them.
struct Layer {
    // opacity, output mode (0 straight, 1 premultiplied, 2 premultiplied toward white), mask on, mask invert
    basic: vec4<f32>,
    // key colour (rgb) and how much to remove (below 0: no key)
    key: vec4<f32>,
    // key edge softness, fringe removal
    key_options: vec4<f32>,
    // frame half width, half height, corner radius, edge fade, in pixels (x below 0: no frame)
    frame: vec4<f32>,
    // half width and height of the layer in pixels (for masks and wipe patterns)
    half: vec4<f32>,
    // brightness, contrast, saturation, hue (radians)
    colour: vec4<f32>,
    // gamma, look-up table strength (0: none), sharpen, colour adjusted (0/1)
    colour2: vec4<f32>,
    // luma key on (0/1), lowest kept, highest kept, softness
    luma: vec4<f32>,
    // scroll offset (x, y) in layer sizes, texel width, texel height
    scroll: vec4<f32>,
    // the cropped part of the source being shown: u0, v0, u1, v1
    crop: vec4<f32>,
    // colour multiplied in (solid colour quads and fades)
    tint: vec4<f32>,
    // wipe pattern (0: none), progress, softness, unused
    wipe: vec4<f32>,
};
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
@group(1) @binding(0) var<storage, read> layers: array<Layer>;
@group(2) @binding(0) var lut: texture_3d<f32>;
@group(2) @binding(1) var mask: texture_2d<f32>;
@group(2) @binding(2) var extra_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) @interpolate(flat) layer: u32,
};
@vertex fn vertex_main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) local: vec2<f32>, @location(3) layer: u32) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(position, 0., 1.);
    out.uv = uv;
    out.local = local;
    out.layer = layer;
    return out;
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
// Green screen: how far a colour leans the same way as the key colour, away from grey.
fn key_score(rgb: vec3<f32>, key: vec3<f32>) -> f32 {
    let lean = key - vec3<f32>((key.r + key.g + key.b) / 3.);
    let strength = max(dot(lean, lean), 0.000001);
    return dot(rgb - vec3<f32>((rgb.r + rgb.g + rgb.b) / 3.), lean) / strength;
}
// Distance from a point to the edge of a rounded box centred on the layer (negative inside).
fn frame_distance(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.))) + min(max(q.x, q.y), 0.) - radius;
}
// Turns a colour around the grey axis by `angle` (radians), keeping its brightness.
fn rotate_hue(c: vec3<f32>, angle: f32) -> vec3<f32> {
    let k = vec3<f32>(0.57735);
    let cos_a = cos(angle);
    return c * cos_a + cross(k, c) * sin(angle) + k * dot(k, c) * (1. - cos_a);
}
// Wipe patterns, 0…1 across the picture; the old scene stays where the pattern is above progress.
fn wipe_pattern(kind: f32, p: vec2<f32>) -> f32 {
    // p runs -1…1 across the picture.
    let k = i32(kind + 0.5);
    if k == 1 { return p.x * 0.5 + 0.5; }
    if k == 2 { return p.y * 0.5 + 0.5; }
    if k == 3 { return clamp(length(p) / 1.4143, 0., 1.); }
    if k == 4 { return atan2(p.x, -p.y) / 6.28319 + 0.5; }
    if k == 5 { return clamp((abs(p.x) + abs(p.y)) / 2., 0., 1.); }
    if k == 6 { return fract((p.x * 0.5 + 0.5) * 8.); }
    if k == 7 { return clamp(max(abs(p.x), abs(p.y)), 0., 1.); }
    return 0.;
}

@fragment fn fragment_main(input: VertexOut) -> @location(0) vec4<f32> {
    let l = layers[input.layer];
    var uv = input.uv;
    // Scrolling wraps within the part of the source that's shown.
    if l.scroll.x != 0. || l.scroll.y != 0. {
        let size = max(l.crop.zw - l.crop.xy, vec2<f32>(0.0001));
        uv = l.crop.xy + fract((uv - l.crop.xy) / size + l.scroll.xy) * size;
    }
    var rgba = textureSampleLevel(image, image_sampler, uv, 0.);
    if l.colour2.z > 0. {
        // Sharpen: push each pixel away from the average of its four neighbours.
        let dx = vec2<f32>(l.scroll.z, 0.);
        let dy = vec2<f32>(0., l.scroll.w);
        let around = (textureSampleLevel(image, image_sampler, uv + dx, 0.) + textureSampleLevel(image, image_sampler, uv - dx, 0.)
            + textureSampleLevel(image, image_sampler, uv + dy, 0.) + textureSampleLevel(image, image_sampler, uv - dy, 0.)) * 0.25;
        rgba = vec4<f32>(max(rgba.rgb + (rgba.rgb - around.rgb) * l.colour2.z * 2., vec3<f32>(0.)), rgba.a);
    }
    var rgb = rgba.rgb * l.tint.rgb;
    var alpha = rgba.a * l.basic.x * l.tint.a;
    if l.key.w >= 0. {
        let score = key_score(rgb, l.key.rgb);
        let softness = max(l.key_options.x, 0.001);
        let edge = 1. - l.key.w;
        alpha *= 1. - smoothstep(edge - softness, edge, score);
        let lean = l.key.rgb - vec3<f32>((l.key.r + l.key.g + l.key.b) / 3.);
        rgb = max(rgb - lean * clamp(score, 0., 1.) * l.key_options.y, vec3<f32>(0.));
    }
    // Colour work happens on the picture as it looks (gamma-encoded), the way people judge colour.
    if l.colour2.w > 0. || l.colour2.y > 0. || l.luma.x > 0. {
        var c = pow(clamp(rgb, vec3<f32>(0.), vec3<f32>(1.)), vec3<f32>(1. / 2.2));
        if l.colour2.w > 0. {
            c = (c - 0.5) * (1. + l.colour.y) + 0.5 + l.colour.x;
            c = mix(vec3<f32>(luminance(c)), c, l.colour.z);
            if l.colour.w != 0. { c = rotate_hue(c, l.colour.w); }
            c = pow(clamp(c, vec3<f32>(0.), vec3<f32>(1.)), vec3<f32>(1. / max(l.colour2.x, 0.01)));
        }
        if l.colour2.y > 0. {
            let graded = textureSampleLevel(lut, extra_sampler, clamp(c, vec3<f32>(0.), vec3<f32>(1.)), 0.).rgb;
            c = mix(c, graded, l.colour2.y);
        }
        if l.luma.x > 0. {
            let y = luminance(c);
            let soft = max(l.luma.w, 0.001);
            alpha *= smoothstep(l.luma.y - soft, l.luma.y, y) * (1. - smoothstep(l.luma.z, l.luma.z + soft, y));
        }
        rgb = pow(clamp(c, vec3<f32>(0.), vec3<f32>(1.)), vec3<f32>(2.2));
    }
    if l.basic.z > 0. {
        let mask_uv = input.local / max(l.half.xy * 2., vec2<f32>(1.)) + 0.5;
        let m = textureSampleLevel(mask, extra_sampler, mask_uv, 0.);
        var amount = luminance(m.rgb) * m.a;
        if l.basic.w > 0. { amount = 1. - amount; }
        alpha *= amount;
    }
    if l.frame.x >= 0. {
        // Rounding the corners by the fade width too keeps the faded edge free of diagonal creases.
        let radius = min(l.frame.z + l.frame.w, min(l.frame.x, l.frame.y));
        let d = frame_distance(input.local, l.frame.xy, radius);
        alpha *= 1. - smoothstep(-max(l.frame.w, 0.75), 0., d);
    }
    if l.wipe.x > 0. {
        let w = wipe_pattern(l.wipe.x, input.local / max(l.half.xy, vec2<f32>(1.)));
        let soft = max(l.wipe.z, 0.001);
        let t = l.wipe.y * (1. + soft);
        alpha *= smoothstep(t - soft, t, w);
    }
    if l.basic.y > 1.5 {
        // Darken: blend toward white where transparent, so see-through parts leave what's below alone.
        return vec4<f32>(mix(vec3<f32>(1.), rgb, alpha), alpha);
    }
    if l.basic.y > 0.5 {
        return vec4<f32>(rgb * alpha, alpha);
    }
    return vec4<f32>(rgb, alpha);
}
