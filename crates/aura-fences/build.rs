fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") { return; }
    println!("cargo:rerun-if-changed=resources/aura-fences.rc");
    println!("cargo:rerun-if-changed=resources/aura-fences.ico");
    println!("cargo:rerun-if-changed=resources/aura-fences.exe.manifest");
    embed_resource::compile("resources/aura-fences.rc", embed_resource::NONE)
        .manifest_required()
        .expect("failed to embed the Fences Windows application manifest");
}
