//! wasm32-unknown-unknown で sysinfo 0.40.3 がコンパイルできるか、また実行時に何を返すかの確認用。
//! ブラウザ実行はできないため、コンパイル可否とソース上の挙動（unknown backend は Error::Unsupported）を証拠とする。
fn main() {
    match sysinfo::System::new_all() {
        Ok(sys) => println!("cpus={} processes={}", sys.cpus().len(), sys.processes().len()),
        Err(e) => println!("System::new_all() -> Err({e:?})"),
    }
}
