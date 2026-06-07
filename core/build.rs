fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Protobuf コード生成（specta 属性なし＝core は Tauri/TS 非依存）
    prost_build::Config::new()
        .out_dir("src/protocol")
        .compile_protos(&["src/protocol/pb.proto"], &["src/protocol/"])?;
    println!("cargo:rerun-if-changed=src/protocol/pb.proto");
    Ok(())
}
