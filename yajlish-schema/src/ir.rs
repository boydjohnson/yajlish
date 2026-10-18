//! The schema tables the macro generates and the runtime interprets.
//!
//! Every type here is built from `'static` data so that the macro can emit
//! the whole schema set as one `static` array of [`Node`]s. Nodes refer to
//! each other by index ([`NodeId`]), so recursive schemas are cycles in the
//! table.

/// Index of a node in the table.
pub type NodeId = usize;

/// One schema location: a JSON Schema object or a boolean schema.
#[derive(Debug)]
pub struct Node {
    /// Where the node came from, e.g. `point.schema.json#/properties/x`.
    pub path: &'static str,
    /// What the node asserts.
    pub kind: NodeKind,
}

/// A boolean schema or a schema object.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum NodeKind {
    /// `true`: anything is valid.
    True,
    /// `false`: nothing is valid.
    False,
    /// A schema object.
    Schema(Schema),
}

/// The JSON types a `type` keyword can allow, as a bit set.
pub mod types {
    /// `"null"`
    pub const NULL: u8 = 1;
    /// `"boolean"`
    pub const BOOLEAN: u8 = 1 << 1;
    /// `"integer"`: also numbers with a zero fractional part.
    pub const INTEGER: u8 = 1 << 2;
    /// `"number"`: any number.
    pub const NUMBER: u8 = 1 << 3;
    /// `"string"`
    pub const STRING: u8 = 1 << 4;
    /// `"array"`
    pub const ARRAY: u8 = 1 << 5;
    /// `"object"`
    pub const OBJECT: u8 = 1 << 6;
    /// No `type` keyword.
    pub const ANY: u8 = 0x7F;
}

/// The keywords of one schema object. Absent keywords are `None` or empty.
#[derive(Debug)]
pub struct Schema {
    /// Allowed types, see [`types`].
    pub types: u8,
    /// `enum`
    pub enum_: Option<&'static [Literal]>,
    /// `const`
    pub const_: Option<&'static Literal>,
    /// `$ref`
    pub ref_: Option<NodeId>,
    /// `allOf`
    pub all_of: &'static [NodeId],
    /// `anyOf`
    pub any_of: &'static [NodeId],
    /// `oneOf`
    pub one_of: &'static [NodeId],
    /// `not`
    pub not: Option<NodeId>,
    /// `properties`, as decoded property names.
    pub properties: &'static [(&'static str, NodeId)],
    /// `required`
    pub required: &'static [&'static str],
    /// `additionalProperties`
    pub additional_properties: Option<NodeId>,
    /// `prefixItems`
    pub prefix_items: &'static [NodeId],
    /// `items`: applies to the elements after `prefixItems`.
    pub items: Option<NodeId>,
    /// `minItems`
    pub min_items: Option<u64>,
    /// `maxItems`
    pub max_items: Option<u64>,
    /// `minLength`, in Unicode scalar values.
    pub min_length: Option<u64>,
    /// `maxLength`, in Unicode scalar values.
    pub max_length: Option<u64>,
    /// `pattern`, as an index into [`Config::patterns`](crate::__private::Config).
    pub pattern: Option<usize>,
    /// `minimum`
    pub minimum: Option<Num>,
    /// `maximum`
    pub maximum: Option<Num>,
    /// `exclusiveMinimum`
    pub exclusive_minimum: Option<Num>,
    /// `exclusiveMaximum`
    pub exclusive_maximum: Option<Num>,
    /// `multipleOf`
    pub multiple_of: Option<Num>,
}

impl Schema {
    /// A schema object with no keywords, for `..Schema::EMPTY`.
    pub const EMPTY: Schema = Schema {
        types: types::ANY,
        enum_: None,
        const_: None,
        ref_: None,
        all_of: &[],
        any_of: &[],
        one_of: &[],
        not: None,
        properties: &[],
        required: &[],
        additional_properties: None,
        prefix_items: &[],
        items: None,
        min_items: None,
        max_items: None,
        min_length: None,
        max_length: None,
        pattern: None,
        minimum: None,
        maximum: None,
        exclusive_minimum: None,
        exclusive_maximum: None,
        multiple_of: None,
    };
}

/// A number in a schema.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Num {
    /// Fits in `i64`.
    Int(i64),
    /// Anything else.
    Float(f64),
}

/// A JSON value written in a schema, for `enum` and `const`.
#[derive(Debug, PartialEq)]
pub enum Literal {
    /// `null`
    Null,
    /// `true` / `false`
    Bool(bool),
    /// A number.
    Num(Num),
    /// A string, decoded.
    Str(&'static str),
    /// An array.
    Array(&'static [Literal]),
    /// An object.
    Object(&'static [(&'static str, Literal)]),
}

/// One step of a record selector (`at = ".features[0]"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSeg {
    /// An object key, decoded.
    Key(&'static str),
    /// An array index.
    Index(u64),
}
