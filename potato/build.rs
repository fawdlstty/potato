use std::{io::Write as _, path::Path};

fn generated_protobuf() -> anyhow::Result<()> {
    let mod_path = "src/generated/protobuf/mod.rs";
    let mod_content = r#"pub mod perftools_profiles {
    //tonic::include_proto!("perftools.profiles.rs");
    include!("perftools.profiles.rs");
}"#;
    std::fs::create_dir_all("src/generated/protobuf")?;
    if !std::fs::exists(mod_path)? {
        if let Ok(mut file) = std::fs::File::create("src/generated/protobuf/mod.rs") {
            _ = file.write_all(mod_content.as_bytes());
        }
    }
    //
    std::env::set_var("PROTOC", format!("/usr/bin/protoc"));
    std::env::set_var("OUT_DIR", format!("src/generated/protobuf/"));
    let proto_path = format!("../pprof/");
    tonic_build::configure()
        .build_client(false)
        .build_server(false)
        .compile_protos(&[Path::new("profile.proto")], &[Path::new(&proto_path)])?;
    println!("cargo:rerun-if-changed=../pprof/profile.proto");
    Ok(())
}

fn main() {
    if let Err(err) = generated_protobuf() {
        panic!("generated protobuf failed: {err}");
    }
}
