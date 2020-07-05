/// The Status that each Handler method returns.
pub enum Status {
    /// Continue calling methods on the Handler.
    Continue,
    /// Stop calling methods.
    Abort,
}

/// The context passed to each Handler function. Context gives
/// basic information about where it is at in the json document.
pub struct Context {}

impl Context {
    /// The number of left brackets ([) encountered without
    /// corresponding right brackets, at this point in the parse.
    pub fn num_open_brackets(&self) -> usize {
        unimplemented!();
    }

    /// The number of left braces ({) encountered without
    /// corresponding right braces, at this point in the parse.
    pub fn num_open_braces(&self) -> usize {
        unimplemented!();
    }
}

/// Implement this trait to handle parse events.
pub trait Handler {
    /// Latest parsed value was a null.
    fn handle_null(&mut self, ctx: &Context) -> Status;

    /// Latest parsed value was a double.
    fn handle_double(&mut self, ctx: &Context, val: f64) -> Status;

    /// Latest parsed value was an int.
    fn handle_int(&mut self, ctx: &Context, val: i64) -> Status;

    /// Latest parsed value was a bool.
    fn handle_bool(&mut self, ctx: &Context, val: bool) -> Status;

    /// Latest parsed value was a string.
    fn handle_string(&mut self, ctx: &Context, val: &str) -> Status;

    /// Latest parsed value was a left curly brace ({).
    fn handle_start_map(&mut self, ctx: &Context) -> Status;

    /// Latest parsed value was a right curly brace (}).
    fn handle_end_map(&mut self, ctx: &Context) -> Status;

    /// Latest parsed value was a key to a JSON object.
    fn handle_map_key(&mut self, ctx: &Context, key: &str) -> Status;

    /// Latest parsed value was a left bracket ([).
    fn handle_start_array(&mut self, ctx: &Context) -> Status;

    /// Latest parsed value was a right bracket ([).
    fn handle_end_array(&mut self, ctx: &Context) -> Status;
}
