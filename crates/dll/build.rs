fn main() {
    #[cfg(target_env = "msvc")]
    static_vcruntime::metabuild();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    println!("cargo:rerun-if-changed=native/memory_reader_bridge.cpp");
    println!("cargo:rerun-if-changed=native/memory_reader_bridge.hpp");
    println!("cargo:rerun-if-changed=../../native/yourgg-memory-reader/include");
    println!("cargo:rerun-if-changed=../../native/yourgg-memory-reader/src/memory_reader.cpp");

    cc::Build::new()
        .cpp(true)
        .std("c++20")
        .include("native")
        .include("../../native/yourgg-memory-reader/include")
        .file("native/memory_reader_bridge.cpp")
        .file("../../native/yourgg-memory-reader/src/memory_reader.cpp")
        .flag_if_supported("/EHsc")
        .compile("yourgg_memory_reader");
}
