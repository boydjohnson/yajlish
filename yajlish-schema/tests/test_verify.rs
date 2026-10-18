//! Bolero harnesses and exhaustive tests for the schema runtime (Plan 4,
//! K5), see `plans/04-kani-bolero.md` in the workspace root. The runtime
//! keeps its state in `Vec`s and `String`s and decodes keys with
//! `decode_str`, which is beyond what Kani finishes in this workspace, so
//! these are not Kani proofs; the number code's proofs are in `src/verify.rs`.

mod support;

use serde_json::{json, Map, Value};
use std::sync::OnceLock;
use yajlish::{Context, Handler, Parser, Status};
use yajlish_schema::__private::{Config, Raw, Runtime};

// `support::config` leaks each `Config` (with `Box::leak`) to get a
// `&'static` one. The configs here are built once and kept in statics: one
// built per fuzz input, or held only by a local, ends up unreachable, and
// libFuzzer's LeakSanitizer reports it as a leak, failing the target.

/// An object with a required integer `n` and an optional string `s`.
fn small_config() -> &'static Config {
    static CONFIG: OnceLock<&'static Config> = OnceLock::new();
    CONFIG.get_or_init(|| {
        support::config(
            &[json!({
                "type": "object",
                "required": ["n"],
                "properties": {"n": {"type": "integer"}, "s": {"type": "string"}}
            })],
            false,
        )
        .unwrap()
    })
}

/// One of the events the runtime can receive, keys and strings raw. Includes
/// a lone surrogate, which `decode_str` rejects.
fn event(pick: u8) -> Raw<'static> {
    match pick % 14 {
        0 => Raw::Null,
        1 => Raw::Bool(true),
        2 => Raw::Int(1),
        3 => Raw::Int(i64::MIN),
        4 => Raw::Float(1.5),
        5 => Raw::Str("\"x\""),
        6 => Raw::Str("\"\\ud800\""),
        7 => Raw::Key("\"n\""),
        8 => Raw::Key("\"s\""),
        9 => Raw::Key("\"\\ud800\""),
        10 => Raw::StartMap,
        11 => Raw::EndMap,
        12 => Raw::StartArray,
        _ => Raw::EndArray,
    }
}

/// `Runtime::event` on any sequence of events, balanced or not, then
/// `finish`. Nothing may panic.
fn check_runtime_never_panics(picks: &[u8]) {
    let mut rt = Runtime::new(small_config());
    for (offset, &pick) in picks.iter().enumerate() {
        let _ = rt.event(offset as u64, event(pick));
    }
    let _ = rt.finish();
}

#[test]
fn runtime_never_panics() {
    bolero::check!()
        .with_type::<Vec<u8>>()
        .for_each(|picks| check_runtime_never_panics(picks));
}

/// Every sequence of up to 5 events.
#[test]
fn exhaustive_runtime_events() {
    fn go(picks: &mut Vec<u8>, left: usize) {
        check_runtime_never_panics(picks);
        if left == 0 {
            return;
        }
        for pick in 0..14 {
            picks.push(pick);
            go(picks, left - 1);
            picks.pop();
        }
    }
    go(&mut vec![], 5);
}

/// The schemas of `tests/test_differential.rs`, which `jsonschema` can load
/// on their own.
const SCHEMAS: &[&str] = &[
    "tree.schema.json",
    "point_feature.schema.json",
    "polygon_feature.schema.json",
    "loose.schema.json",
    "strict.schema.json",
];

const KEYS: &[&str] = &[
    "label",
    "children",
    "shape",
    "w",
    "h",
    "type",
    "geometry",
    "coordinates",
    "properties",
    "n",
    "extra",
];

const STRINGS: &[&str] = &["", "x", "Feature", "Point", "Polygon"];

fn load(name: &str) -> Value {
    let path = format!("{}/tests/schemas/{}", env!("CARGO_MANIFEST_DIR"), name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// A JSON value chosen by `choices`, from the schemas' own vocabulary so
/// that some instances are valid.
fn gen_value(choices: &mut impl Iterator<Item = u8>, depth: u32) -> Value {
    let c = choices.next().unwrap_or(0);
    match c % 8 {
        0 => Value::Null,
        1 => Value::Bool(c & 0x10 != 0),
        2 => json!(i64::from(c % 13) - 3),
        3 => json!([-0.5, 0.0, 1.0, 2.5, 1e300][usize::from(c) % 5]),
        4 => json!(STRINGS[usize::from(c) % STRINGS.len()]),
        5 if depth > 0 => {
            let n = choices.next().unwrap_or(0) % 5;
            Value::Array((0..n).map(|_| gen_value(choices, depth - 1)).collect())
        }
        6 | 7 if depth > 0 => {
            let n = choices.next().unwrap_or(0) % 5;
            let mut m = Map::new();
            for _ in 0..n {
                let k = KEYS[usize::from(choices.next().unwrap_or(0)) % KEYS.len()];
                m.insert(k.into(), gen_value(choices, depth - 1));
            }
            Value::Object(m)
        }
        _ => json!("x"),
    }
}

/// Forwards to a `Runtime` and checks, after every event, that the number
/// of live candidates never grows within a record: it only drops, or goes
/// from 0 to all schemas when a record starts.
struct Watch {
    rt: Runtime,
    live: usize,
}

impl Watch {
    fn ev(&mut self, ctx: &Context, raw: Raw<'_>) -> Status {
        let _ = self.rt.event(ctx.offset(), raw);
        let live = self.rt.live_candidates();
        assert!(
            self.live == 0 || live <= self.live,
            "candidates grew from {} to {}",
            self.live,
            live
        );
        self.live = live;
        Status::Continue
    }
}

impl Handler for Watch {
    fn handle_null(&mut self, ctx: &Context) -> Status {
        self.ev(ctx, Raw::Null)
    }
    fn handle_double(&mut self, ctx: &Context, v: f64) -> Status {
        self.ev(ctx, Raw::Float(v))
    }
    fn handle_int(&mut self, ctx: &Context, v: i64) -> Status {
        self.ev(ctx, Raw::Int(v))
    }
    fn handle_bool(&mut self, ctx: &Context, v: bool) -> Status {
        self.ev(ctx, Raw::Bool(v))
    }
    fn handle_string(&mut self, ctx: &Context, v: &str) -> Status {
        self.ev(ctx, Raw::Str(v))
    }
    fn handle_start_map(&mut self, ctx: &Context) -> Status {
        self.ev(ctx, Raw::StartMap)
    }
    fn handle_end_map(&mut self, ctx: &Context) -> Status {
        self.ev(ctx, Raw::EndMap)
    }
    fn handle_map_key(&mut self, ctx: &Context, v: &str) -> Status {
        self.ev(ctx, Raw::Key(v))
    }
    fn handle_start_array(&mut self, ctx: &Context) -> Status {
        self.ev(ctx, Raw::StartArray)
    }
    fn handle_end_array(&mut self, ctx: &Context) -> Status {
        self.ev(ctx, Raw::EndArray)
    }
}

/// `live_candidates()` never increases within a record, with all the
/// schemas at once, with and without discriminator dispatch.
#[test]
fn candidates_only_shrink() {
    static CONFIGS: OnceLock<[&'static Config; 2]> = OnceLock::new();
    let configs = *CONFIGS.get_or_init(|| {
        let schemas: Vec<Value> = SCHEMAS.iter().map(|s| load(s)).collect();
        [
            support::config_with(&schemas, false, true).unwrap(),
            support::config_with(&schemas, false, false).unwrap(),
        ]
    });
    bolero::check!().with_type::<Vec<u8>>().for_each(|choices| {
        let text = serde_json::to_string(&gen_value(&mut choices.iter().copied(), 4)).unwrap();
        for cfg in configs {
            let mut watch = Watch {
                rt: Runtime::new(cfg),
                live: 0,
            };
            let mut parser = Parser::new(&mut watch);
            parser.parse(&mut text.as_bytes()).unwrap();
            parser.finish_parse().unwrap();
        }
    });
}

/// With `match = unique` (`match_first: false`), the outcome is `Ok(i)`
/// exactly when `jsonschema` accepts the instance against schema `i` and
/// against no other. The fuzz version of `tests/test_differential.rs`, with
/// all the schemas in one handler.
#[test]
fn unique_means_exactly_one() {
    // In a static: bolero needs the test closure to be `RefUnwindSafe`,
    // and `jsonschema`'s validators are not.
    static VALIDATORS: OnceLock<Vec<jsonschema::Validator>> = OnceLock::new();
    let schemas: Vec<Value> = SCHEMAS.iter().map(|s| load(s)).collect();
    VALIDATORS.get_or_init(|| {
        schemas
            .iter()
            .map(|s| jsonschema::validator_for(s).unwrap())
            .collect()
    });
    static CONFIG: OnceLock<&'static Config> = OnceLock::new();
    let cfg = *CONFIG.get_or_init(|| support::config(&schemas, false).unwrap());
    bolero::check!()
        .with_type::<(Vec<u8>, u8)>()
        .for_each(|(choices, chunk)| {
            let instance = gen_value(&mut choices.iter().copied(), 4);
            let accepted: Vec<usize> = VALIDATORS
                .get()
                .unwrap()
                .iter()
                .enumerate()
                .filter(|(_, v)| v.is_valid(&instance))
                .map(|(i, _)| i)
                .collect();
            let text = serde_json::to_string(&instance).unwrap();
            let chunk = usize::from(*chunk).max(1);
            let ours = support::validate_text(cfg, text.as_bytes(), chunk);
            match (&accepted[..], &ours) {
                ([only], Ok(i)) => assert_eq!(only, i, "{}", text),
                ([_], Err(e)) => panic!("{}: jsonschema accepts one schema, ours: {:?}", text, e),
                (_, Ok(i)) => panic!(
                    "{}: ours chose {}, jsonschema accepts {:?}",
                    text, i, accepted
                ),
                (_, Err(_)) => {}
            }
        });
}
