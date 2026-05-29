fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/fbi_agent.proto");
    println!("cargo:rerun-if-changed=build.rs");
    tonic_prost_build::compile_protos("proto/fbi_agent.proto")?;
    Ok(())
}
