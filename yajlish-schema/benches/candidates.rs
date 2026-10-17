//! Three discriminated schemas should cost about the same as one: the
//! other candidates are dropped as soon as `geometry.type` is read.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use yajlish::{Handler, Parser};
use yajlish_schema::Status;

mod one {
    yajlish_schema::handler! {
        pub struct FeatureHandler;
        at = ".features";
        build = false;
        schemas {
            Point => "tests/schemas/point_feature.schema.json",
        }
    }
}

mod three {
    yajlish_schema::handler! {
        pub struct FeatureHandler;
        at = ".features";
        build = false;
        schemas {
            Point => "tests/schemas/point_feature.schema.json",
            Polygon => "tests/schemas/polygon_feature.schema.json",
            MultiPolygon => "tests/schemas/multipolygon_feature.schema.json",
        }
    }
}

mod three_built {
    yajlish_schema::handler! {
        pub struct FeatureHandler;
        at = ".features";
        schemas {
            Point => "tests/schemas/point_feature.schema.json",
            Polygon => "tests/schemas/polygon_feature.schema.json",
            MultiPolygon => "tests/schemas/multipolygon_feature.schema.json",
        }
    }
}

/// Counts parse events and does nothing else: the floor.
struct Count(u64);

impl Handler for Count {
    fn handle_null(&mut self, _: &yajlish::Context) -> Status {
        self.0 += 1;
        Status::Continue
    }
    fn handle_double(&mut self, _: &yajlish::Context, _: f64) -> Status {
        self.0 += 1;
        Status::Continue
    }
    fn handle_int(&mut self, _: &yajlish::Context, _: i64) -> Status {
        self.0 += 1;
        Status::Continue
    }
    fn handle_bool(&mut self, _: &yajlish::Context, _: bool) -> Status {
        self.0 += 1;
        Status::Continue
    }
    fn handle_string(&mut self, _: &yajlish::Context, _: &str) -> Status {
        self.0 += 1;
        Status::Continue
    }
    fn handle_start_map(&mut self, _: &yajlish::Context) -> Status {
        self.0 += 1;
        Status::Continue
    }
    fn handle_end_map(&mut self, _: &yajlish::Context) -> Status {
        self.0 += 1;
        Status::Continue
    }
    fn handle_map_key(&mut self, _: &yajlish::Context, _: &str) -> Status {
        self.0 += 1;
        Status::Continue
    }
    fn handle_start_array(&mut self, _: &yajlish::Context) -> Status {
        self.0 += 1;
        Status::Continue
    }
    fn handle_end_array(&mut self, _: &yajlish::Context) -> Status {
        self.0 += 1;
        Status::Continue
    }
}

fn features() -> Vec<u8> {
    let mut s = String::from("{\"type\": \"FeatureCollection\", \"features\": [");
    for i in 0..5000 {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"type\": \"Feature\", \"id\": {}, \"geometry\": {{\"type\": \"Point\", \"coordinates\": [{}.5, -{}.25]}}, \
             \"properties\": {{\"name\": \"feature {}\", \"rank\": {}}}}}",
            i, i, i, i, i % 7
        ));
    }
    s.push_str("]}");
    s.into_bytes()
}

macro_rules! bench_handler {
    ($group:expr, $name:expr, $input:expr, $m:ident) => {
        $group.bench_function($name, |b| {
            b.iter(|| {
                let mut ok = 0u32;
                let mut handler = $m::FeatureHandler::new(|rec| {
                    if rec.is_ok() {
                        ok += 1;
                    }
                    Status::Continue
                });
                let mut parser = Parser::new(&mut handler);
                parser.parse(&mut black_box($input.as_slice())).unwrap();
                parser.finish_parse().unwrap();
                handler.finish().unwrap();
                assert_eq!(ok, 5000);
            })
        });
    };
}

fn bench(c: &mut Criterion) {
    let input = features();
    let mut group = c.benchmark_group("5000 point features");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_function("parse only", |b| {
        b.iter(|| {
            let mut h = Count(0);
            let mut p = Parser::new(&mut h);
            p.parse(&mut black_box(input.as_slice())).unwrap();
            h.0
        })
    });
    bench_handler!(group, "validate, 1 schema", input, one);
    bench_handler!(group, "validate, 3 schemas", input, three);
    bench_handler!(group, "validate and build, 3 schemas", input, three_built);
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
