struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
    @location(2) key: vec4<f32>,
    @location(3) key_options: vec2<f32>,
};
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
@vertex fn vertex_main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) opacity: f32, @location(3) key: vec4<f32>, @location(4) key_options: vec2<f32>) -> VertexOut {
    var output: VertexOut;
    output.position=vec4<f32>(position,0.,1.);output.uv=uv;output.opacity=opacity;
    output.key=key;output.key_options=key_options;return output;
}
fn chroma(rgb:vec3<f32>) -> vec2<f32> {return vec2<f32>(dot(rgb,vec3<f32>(-0.169,-0.331,0.5)),dot(rgb,vec3<f32>(0.5,-0.419,-0.081)));}
@fragment fn fragment_main(input: VertexOut) -> @location(0) vec4<f32> {
    let rgba=textureSample(image,image_sampler,input.uv);
    var alpha=rgba.a*input.opacity;var rgb=rgba.rgb;
    if input.key.w>=0. {
        let distance=length(chroma(rgb)-chroma(input.key.rgb));
        let mask=smoothstep(input.key.w,input.key.w+max(input.key_options.x,0.001),distance);
        alpha*=mask;
        let edge=1.-smoothstep(input.key.w,input.key.w+max(input.key_options.x,0.001)*3.,distance);
        let gray=dot(rgb,vec3<f32>(0.2126,0.7152,0.0722));
        rgb=mix(rgb,vec3<f32>(gray),edge*input.key_options.y);
    }
    return vec4<f32>(rgb,alpha);
}
