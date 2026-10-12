@group(0) @binding(0) var atlas: texture_2d<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) textured: f32,
}

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) textured: f32,
}

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    var output: VertexOut;
    output.position = vec4<f32>(input.position, 0.0, 1.0);
    output.color = input.color;
    output.uv = input.uv;
    output.textured = input.textured;
    return output;
}

struct BootstrapVertexIn {
    @location(0) position: vec2<f32>,
    @location(1) unused_color: vec4<f32>,
    @location(2) unused_uv: vec2<f32>,
    @location(3) unused_textured: f32,
    @location(4) translation: vec2<f32>,
    @location(5) color: vec4<f32>,
}

@vertex
fn vs_bootstrap(input: BootstrapVertexIn) -> VertexOut {
    var output: VertexOut;
    output.position = vec4<f32>(input.position + input.translation, 0.0, 1.0);
    output.color = input.color;
    output.uv = vec2<f32>(0.0, 0.0);
    output.textured = 0.0;
    return output;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    if input.textured > 0.5 {
        return textureSample(atlas, atlas_sampler, input.uv);
    }
    return input.color;
}
