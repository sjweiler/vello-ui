use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo::rerun-if-changed=shaders/ui.wgsl");
    let source = fs::read_to_string("shaders/ui.wgsl").expect("read Ash UI shader");
    let module = naga::front::wgsl::parse_str(&source).expect("parse Ash UI shader");
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("validate Ash UI shader");
    let options = naga::back::spv::Options::default();
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    for (entry, stage, filename) in [
        ("vs_main", naga::ShaderStage::Vertex, "ui.vert.spv"),
        (
            "vs_bootstrap",
            naga::ShaderStage::Vertex,
            "ui.bootstrap.vert.spv",
        ),
        ("fs_main", naga::ShaderStage::Fragment, "ui.frag.spv"),
    ] {
        let words = naga::back::spv::write_vec(
            &module,
            &info,
            &options,
            Some(&naga::back::spv::PipelineOptions {
                entry_point: entry.to_owned(),
                shader_stage: stage,
            }),
        )
        .expect("compile Ash UI shader");
        let bytes: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
        fs::write(output.join(filename), bytes).expect("write compiled Ash UI shader");
    }
}
