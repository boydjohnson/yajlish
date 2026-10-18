//! Compares this parser with the 0.4 release (json-tools based) on the test
//! fixtures and on a larger generated document.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

/// Counts events; implements both versions' `Handler`.
#[derive(Default)]
struct Count(u64);

macro_rules! impl_count {
    ($krate:ident) => {
        impl $krate::Handler for Count {
            fn handle_null(&mut self, _: &$krate::Context) -> $krate::Status {
                self.0 += 1;
                $krate::Status::Continue
            }
            fn handle_double(&mut self, _: &$krate::Context, _: f64) -> $krate::Status {
                self.0 += 1;
                $krate::Status::Continue
            }
            fn handle_int(&mut self, _: &$krate::Context, _: i64) -> $krate::Status {
                self.0 += 1;
                $krate::Status::Continue
            }
            fn handle_bool(&mut self, _: &$krate::Context, _: bool) -> $krate::Status {
                self.0 += 1;
                $krate::Status::Continue
            }
            fn handle_string(&mut self, _: &$krate::Context, s: &str) -> $krate::Status {
                self.0 += s.len() as u64;
                $krate::Status::Continue
            }
            fn handle_start_map(&mut self, _: &$krate::Context) -> $krate::Status {
                self.0 += 1;
                $krate::Status::Continue
            }
            fn handle_end_map(&mut self, _: &$krate::Context) -> $krate::Status {
                self.0 += 1;
                $krate::Status::Continue
            }
            fn handle_map_key(&mut self, _: &$krate::Context, k: &str) -> $krate::Status {
                self.0 += k.len() as u64;
                $krate::Status::Continue
            }
            fn handle_start_array(&mut self, _: &$krate::Context) -> $krate::Status {
                self.0 += 1;
                $krate::Status::Continue
            }
            fn handle_end_array(&mut self, _: &$krate::Context) -> $krate::Status {
                self.0 += 1;
                $krate::Status::Continue
            }
        }
    };
}

impl_count!(yajlish);
impl_count!(yajlish_04);

fn generated() -> Vec<u8> {
    let mut s = String::from("{\"type\": \"FeatureCollection\", \"features\": [");
    for i in 0..5000 {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"type\": \"Feature\", \"id\": {}, \"properties\": {{\"name\": \"feature number {}\", \
             \"active\": {}, \"note\": null}}, \"geometry\": {{\"type\": \"Point\", \
             \"coordinates\": [{}.125, -{}.5e-3]}}}}",
            i,
            i,
            i % 2 == 0,
            i,
            i
        ));
    }
    s.push_str("]}");
    s.into_bytes()
}

fn bench(c: &mut Criterion) {
    let root = env!("CARGO_MANIFEST_DIR");
    let inputs = vec![
        (
            "custom.geo.json",
            std::fs::read(format!(
                "{}/yajlish-ndjson/tests/ndjson_fixtures/custom.geo.json",
                root
            ))
            .unwrap(),
        ),
        (
            "prize.json",
            std::fs::read(format!(
                "{}/yajlish-ndjson/tests/ndjson_fixtures/prize.json",
                root
            ))
            .unwrap(),
        ),
        ("generated", generated()),
    ];

    for (name, input) in &inputs {
        let mut group = c.benchmark_group(*name);
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_function("0.5 logos", |b| {
            b.iter(|| {
                let mut h = Count::default();
                let mut p = yajlish::Parser::new(&mut h);
                p.parse(&mut black_box(input.as_slice())).unwrap();
                p.finish_parse().unwrap();
                h.0
            })
        });
        group.bench_function("0.4 json-tools", |b| {
            b.iter(|| {
                let mut h = Count::default();
                let mut p = yajlish_04::Parser::new(&mut h);
                p.parse(&mut black_box(input.as_slice())).unwrap();
                p.finish_parse().unwrap();
                h.0
            })
        });
        group.finish();
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
