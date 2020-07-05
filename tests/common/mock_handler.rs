use yajlish::{Context, Handler, Status};

pub struct MockHandler<W> {
    write: W,
}

impl<W: std::io::Write> MockHandler<W> {
    pub fn new(out: W) -> Self {
        MockHandler { write: out }
    }
}

impl<W> Handler for MockHandler<W>
where
    W: std::io::Write,
{
    fn handle_bool(&mut self, _ctx: &Context, val: bool) -> Status {
        writeln!(self.write, "bool: {}", val).expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_double(&mut self, _ctx: &Context, val: f64) -> Status {
        writeln!(self.write, "double: {}", val).expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_end_array(&mut self, _ctx: &Context) -> Status {
        writeln!(self.write, "array close ']'").expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_end_map(&mut self, _ctx: &Context) -> Status {
        writeln!(self.write, "map close '}}'").expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_int(&mut self, _ctx: &Context, val: i64) -> Status {
        writeln!(self.write, "integer: {}", val).expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_map_key(&mut self, _ctx: &Context, key: &str) -> Status {
        writeln!(self.write, "key: {}", key).expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_null(&mut self, _ctx: &Context) -> Status {
        writeln!(self.write, "null").expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_start_array(&mut self, _ctx: &Context) -> Status {
        writeln!(self.write, "array open '['").expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_start_map(&mut self, _ctx: &Context) -> Status {
        writeln!(self.write, "map open '{{'").expect("Unable to write to stdout");
        Status::Continue
    }

    fn handle_string(&mut self, _ctx: &Context, val: &str) -> Status {
        writeln!(self.write, "string: '{}'", val).expect("Unable to write to stdout");
        Status::Continue
    }
}
