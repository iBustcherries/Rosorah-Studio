struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
    @location(2) key: vec4<f32>,
    @location(3) key_options: vec2<f32>,
    @location(4) local: vec2<f32>,
    @location(5) frame: vec4<f32>,
};
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
@vertex fn vertex_main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) opacity: f32, @location(3) key: vec4<f32>, @location(4) key_options: vec2<f32>, @location(5) local: vec2<f32>, @location(6) frame: vec4<f32>) -> VertexOut {
    var output: VertexOut;
    output.position=vec4<f32>(position,0.,1.);output.uv=uv;output.opacity=opacity;
    output.key=key;output.key_options=key_options;output.local=local;output.frame=frame;return output;
}
// Green screen: how far a colour leans the same way as the key colour, away from grey.
// The key colour itself scores 1, greys and whites 0, opposite colours below 0.
fn key_score(rgb: vec3<f32>, key: vec3<f32>) -> f32 {
    let lean=key-vec3<f32>((key.r+key.g+key.b)/3.);
    let strength=max(dot(lean,lean),0.000001);
    return dot(rgb-vec3<f32>((rgb.r+rgb.g+rgb.b)/3.),lean)/strength;
}
// Distance from a point to the edge of a rounded box centred on the layer (negative inside).
fn frame_distance(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q=abs(p)-half_size+vec2<f32>(radius);
    return length(max(q,vec2<f32>(0.)))+min(max(q.x,q.y),0.)-radius;
}
@fragment fn fragment_main(input: VertexOut) -> @location(0) vec4<f32> {
    let rgba=textureSample(image,image_sampler,input.uv);
    var alpha=rgba.a*input.opacity;var rgb=rgba.rgb;
    // key.w is how much to remove (below 0: off); options are edge softness and spill removal.
    if input.key.w>=0. {
        let score=key_score(rgb,input.key.rgb);
        let softness=max(input.key_options.x,0.001);
        let edge=1.-input.key.w;
        alpha*=1.-smoothstep(edge-softness,edge,score);
        // Take the key colour's tint back out of what stays (green fringes on hair and edges).
        let lean=input.key.rgb-vec3<f32>((input.key.r+input.key.g+input.key.b)/3.);
        rgb=max(rgb-lean*clamp(score,0.,1.)*input.key_options.y,vec3<f32>(0.));
    }
    // frame: half width, half height, corner radius, edge fade (all in pixels); x below 0 is off.
    if input.frame.x>=0. {
        // Rounding the corners by the fade width too keeps the faded edge free of diagonal creases.
        let radius=min(input.frame.z+input.frame.w,min(input.frame.x,input.frame.y));
        let d=frame_distance(input.local,input.frame.xy,radius);
        let fade=max(input.frame.w,0.75);
        alpha*=1.-smoothstep(-fade,0.,d);
    }
    return vec4<f32>(rgb,alpha);
}
