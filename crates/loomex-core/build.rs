fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "../../proto/loomex/runner/v1/runner_stream.proto";

    println!("cargo:rerun-if-changed={proto}");

    tonic_build::configure()
        .build_server(false)
        .compile_protos(&[proto], &["../../proto"])?;

    Ok(())
}
