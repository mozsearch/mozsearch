fn main() -> Result<(), Box<dyn std::error::Error>> {
    // right now we only use the livegrep proto and not the config proto
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(
            &["../deps/livegrep/src/proto/livegrep.proto"],
            &["../deps/livegrep/"],
        )?;
    Ok(())
}
