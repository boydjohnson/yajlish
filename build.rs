fn main() {
    #[cfg(feature = "ndjson")]
    lalrpop::process_root().unwrap();
}
