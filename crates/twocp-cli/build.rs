use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo should set manifest dir"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo should set OUT_DIR"));
    let providers = [
        ("git-minimal.json", "git-minimal.twocp-provider"),
        ("kubectl-minimal.json", "kubectl-minimal.twocp-provider"),
    ];

    for (source_name, output_name) in providers {
        let source_path = manifest_dir.join("../../providers-src").join(source_name);
        let output_path = out_dir.join(output_name);
        println!("cargo:rerun-if-changed={}", source_path.display());
        twocp_build::compile_json_file_to_file(&source_path, &output_path).unwrap_or_else(
            |error| panic!("failed to compile built-in provider fixture {source_name}: {error}"),
        );
    }
}
