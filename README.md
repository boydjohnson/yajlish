# Rust library for streaming, event-based JSON parsing, based loosely on [yajl](https://github.com/yajl/yajl)

### Libraries you probably need instead of this library
- [rust-json](https://crates.io/crates/json)
- [serde_json](https://crates.io/crates/serde_json)

### Features
- Partial conformance with test cases from yajl
- Parse JSON without loading the whole JSON document into memory

### Use

Suppose you wanted to parse the count of all JSON object keys that are named 'foo'.

```rust
     use yajlish::{Context, Handler, Status};

     pub struct FooCountHandler {
         count: usize,
     }

     impl Handler for FooCountHandler {
         
         fn handle_map_key(&mut self, _ctx: &Context, key: &str) -> Status {
             if key == "foo" {
                 self.count += 1;
             }
             Status::Continue
         }
     
         fn handle_null(&mut self, _ctx: &Context) -> Status {
             Status::Continue
         }

         fn handle_bool(&mut self, _ctx: &Context, boolean: bool) -> Status {
             Status::Continue
         }

         fn handle_double(&mut self, _ctx: &Context, val: f64) -> Status {
             Status::Continue
         }
         
         fn handle_int(&mut self, _ctx: &Context, val: i64) -> Status {
             Status::Continue
         }

         fn handle_string(&mut self, _ctx: &Context, val: &str) -> Status {
             Status::Continue
         }

         fn handle_start_map(&mut self, _ctx: &Context) -> Status {
             Status::Continue
         }
         
         fn handle_start_array(&mut self, _ctx: &Context) -> Status {
             Status::Continue
         }

         fn handle_end_map(&mut self, _ctx: &Context) -> Status {
             Status::Continue
         }

         fn handle_end_array(&mut self, _ctx: &Context) -> Status {
             Status::Continue
         }
     }
```

# License

This library is licensed under the Apache 2.0 License.
