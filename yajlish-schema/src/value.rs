//! A dynamic JSON value, and conversion from it to generated types.

use crate::matcher::Ev;
use std::collections::BTreeMap;

/// A JSON value, built from the event stream. Generated types use it where a
/// schema does not pin down a Rust type (`true`, a `type` list, ...).
///
/// Objects keep their members in document order, duplicates included.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `null`
    Null,
    /// `true` / `false`
    Bool(bool),
    /// A number that fit in `i64`.
    Int(i64),
    /// Any other number.
    Float(f64),
    /// A decoded string.
    String(String),
    /// An array.
    Array(Vec<Value>),
    /// An object.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// The value's JSON type name, for error messages.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Int(_) => "integer",
            Value::Float(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    /// Feed this value to `f` as a sequence of events.
    pub(crate) fn replay(&self, f: &mut dyn FnMut(Ev<'_>)) {
        match self {
            Value::Null => f(Ev::Null),
            Value::Bool(b) => f(Ev::Bool(*b)),
            Value::Int(i) => f(Ev::Int(*i)),
            Value::Float(x) => f(Ev::Float(*x)),
            Value::String(s) => f(Ev::Str(s)),
            Value::Array(items) => {
                f(Ev::StartArray);
                for item in items {
                    item.replay(f);
                }
                f(Ev::EndArray);
            }
            Value::Object(members) => {
                f(Ev::StartMap);
                for (k, v) in members {
                    f(Ev::Key(k));
                    v.replay(f);
                }
                f(Ev::EndMap);
            }
        }
    }
}

/// Builds a `Value` from events.
#[derive(Debug, Default)]
pub(crate) struct ValueBuilder {
    stack: Vec<(Value, Option<String>)>,
    done: Option<Value>,
}

impl ValueBuilder {
    pub(crate) fn event(&mut self, ev: &Ev<'_>) {
        let v = match ev {
            Ev::Null => Value::Null,
            Ev::Bool(b) => Value::Bool(*b),
            Ev::Int(i) => Value::Int(*i),
            Ev::Float(x) => Value::Float(*x),
            Ev::Str(s) => Value::String((*s).to_owned()),
            Ev::StartMap => {
                self.stack.push((Value::Object(vec![]), None));
                return;
            }
            Ev::StartArray => {
                self.stack.push((Value::Array(vec![]), None));
                return;
            }
            Ev::Key(k) => {
                if let Some(top) = self.stack.last_mut() {
                    top.1 = Some((*k).to_owned());
                }
                return;
            }
            Ev::EndMap | Ev::EndArray => match self.stack.pop() {
                Some((v, _)) => v,
                None => return,
            },
        };
        match self.stack.last_mut() {
            Some((Value::Array(items), _)) => items.push(v),
            Some((Value::Object(members), key)) => {
                members.push((key.take().unwrap_or_default(), v));
            }
            Some(_) => {}
            None => self.done = Some(v),
        }
    }

    /// The finished value, once the outermost value has ended.
    pub(crate) fn take(&mut self) -> Option<Value> {
        self.done.take()
    }
}

/// Why a validated value could not be converted to its generated type.
///
/// Validation runs first, so this only happens where a valid JSON value has
/// no Rust representation, such as an `integer` outside the `i64` range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError {
    /// JSON Pointer to the value, relative to the record.
    pub instance_path: String,
    /// What went wrong.
    pub detail: String,
}

impl BuildError {
    /// An error at the current value.
    #[must_use]
    pub fn new(detail: impl Into<String>) -> Self {
        BuildError {
            instance_path: String::new(),
            detail: detail.into(),
        }
    }

    /// Prefix the path with an object key.
    #[must_use]
    pub fn in_key(mut self, key: &str) -> Self {
        let escaped = key.replace('~', "~0").replace('/', "~1");
        self.instance_path = format!("/{}{}", escaped, self.instance_path);
        self
    }

    /// Prefix the path with an array index.
    #[must_use]
    pub fn in_index(mut self, index: usize) -> Self {
        self.instance_path = format!("/{}{}", index, self.instance_path);
        self
    }

    fn expected(what: &str, v: &Value) -> Self {
        BuildError::new(format!("expected {}, found {}", what, v.type_name()))
    }
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "at '{}': {}", self.instance_path, self.detail)
    }
}

impl std::error::Error for BuildError {}

/// Conversion from a validated [`Value`] into a generated type.
pub trait FromValue: Sized {
    /// Convert.
    ///
    /// # Errors
    /// `BuildError` when the value has the wrong shape or does not fit.
    fn from_value(v: Value) -> Result<Self, BuildError>;
}

impl FromValue for Value {
    fn from_value(v: Value) -> Result<Self, BuildError> {
        Ok(v)
    }
}

impl FromValue for () {
    fn from_value(v: Value) -> Result<Self, BuildError> {
        match v {
            Value::Null => Ok(()),
            v => Err(BuildError::expected("null", &v)),
        }
    }
}

impl FromValue for bool {
    fn from_value(v: Value) -> Result<Self, BuildError> {
        match v {
            Value::Bool(b) => Ok(b),
            v => Err(BuildError::expected("boolean", &v)),
        }
    }
}

impl FromValue for i64 {
    fn from_value(v: Value) -> Result<Self, BuildError> {
        match v {
            Value::Int(i) => Ok(i),
            // `1.0` is an integer per the spec. The bounds are the exact
            // f64 values -2^63 and 2^63.
            #[allow(clippy::cast_possible_truncation)]
            Value::Float(x)
                if x.fract() == 0.0
                    && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&x) =>
            {
                Ok(x as i64)
            }
            Value::Float(x) if x.fract() == 0.0 => Err(BuildError::new(format!(
                "integer {} does not fit in i64",
                x
            ))),
            v => Err(BuildError::expected("integer", &v)),
        }
    }
}

impl FromValue for f64 {
    fn from_value(v: Value) -> Result<Self, BuildError> {
        match v {
            #[allow(clippy::cast_precision_loss)]
            Value::Int(i) => Ok(i as f64),
            Value::Float(x) => Ok(x),
            v => Err(BuildError::expected("number", &v)),
        }
    }
}

impl FromValue for String {
    fn from_value(v: Value) -> Result<Self, BuildError> {
        match v {
            Value::String(s) => Ok(s),
            v => Err(BuildError::expected("string", &v)),
        }
    }
}

impl<T: FromValue> FromValue for Option<T> {
    fn from_value(v: Value) -> Result<Self, BuildError> {
        match v {
            Value::Null => Ok(None),
            v => T::from_value(v).map(Some),
        }
    }
}

impl<T: FromValue> FromValue for Box<T> {
    fn from_value(v: Value) -> Result<Self, BuildError> {
        T::from_value(v).map(Box::new)
    }
}

impl<T: FromValue> FromValue for Vec<T> {
    fn from_value(v: Value) -> Result<Self, BuildError> {
        match v {
            Value::Array(items) => items
                .into_iter()
                .enumerate()
                .map(|(i, item)| T::from_value(item).map_err(|e| e.in_index(i)))
                .collect(),
            v => Err(BuildError::expected("array", &v)),
        }
    }
}

impl<T: FromValue> FromValue for BTreeMap<String, T> {
    fn from_value(v: Value) -> Result<Self, BuildError> {
        match v {
            Value::Object(members) => {
                let mut map = BTreeMap::new();
                for (k, v) in members {
                    let v = T::from_value(v).map_err(|e| e.in_key(&k))?;
                    map.insert(k, v);
                }
                Ok(map)
            }
            v => Err(BuildError::expected("object", &v)),
        }
    }
}

/// Split an object into its members, for generated struct conversions.
#[doc(hidden)]
pub fn into_members(v: Value) -> Result<Vec<(String, Value)>, BuildError> {
    match v {
        Value::Object(members) => Ok(members),
        v => Err(BuildError::expected("object", &v)),
    }
}

/// Split an array into exactly `n` elements, for generated tuple structs.
#[doc(hidden)]
pub fn into_elements(v: Value, n: usize) -> Result<std::vec::IntoIter<Value>, BuildError> {
    match v {
        Value::Array(items) if items.len() == n => Ok(items.into_iter()),
        Value::Array(items) => Err(BuildError::new(format!(
            "expected {} elements, found {}",
            n,
            items.len()
        ))),
        v => Err(BuildError::expected("array", &v)),
    }
}

/// Take a decoded string, for generated string enums.
#[doc(hidden)]
pub fn into_string(v: Value) -> Result<String, BuildError> {
    String::from_value(v)
}

#[cfg(feature = "serde")]
mod serde_impls {
    use super::Value;
    use serde::{
        de::{MapAccess, SeqAccess, Visitor},
        ser::{SerializeMap, SerializeSeq},
        Deserialize, Deserializer, Serialize, Serializer,
    };

    impl Serialize for Value {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            match self {
                Value::Null => s.serialize_unit(),
                Value::Bool(b) => s.serialize_bool(*b),
                Value::Int(i) => s.serialize_i64(*i),
                Value::Float(x) => s.serialize_f64(*x),
                Value::String(v) => s.serialize_str(v),
                Value::Array(items) => {
                    let mut seq = s.serialize_seq(Some(items.len()))?;
                    for item in items {
                        seq.serialize_element(item)?;
                    }
                    seq.end()
                }
                Value::Object(members) => {
                    let mut map = s.serialize_map(Some(members.len()))?;
                    for (k, v) in members {
                        map.serialize_entry(k, v)?;
                    }
                    map.end()
                }
            }
        }
    }

    struct ValueVisitor;

    impl<'de> Visitor<'de> for ValueVisitor {
        type Value = Value;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("any JSON value")
        }
        fn visit_unit<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_none<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
            Value::deserialize(d)
        }
        fn visit_bool<E>(self, b: bool) -> Result<Value, E> {
            Ok(Value::Bool(b))
        }
        fn visit_i64<E>(self, i: i64) -> Result<Value, E> {
            Ok(Value::Int(i))
        }
        fn visit_u64<E>(self, u: u64) -> Result<Value, E> {
            #[allow(clippy::cast_precision_loss)]
            Ok(i64::try_from(u).map_or(Value::Float(u as f64), Value::Int))
        }
        fn visit_f64<E>(self, x: f64) -> Result<Value, E> {
            Ok(Value::Float(x))
        }
        fn visit_str<E>(self, v: &str) -> Result<Value, E> {
            Ok(Value::String(v.to_owned()))
        }
        fn visit_string<E>(self, v: String) -> Result<Value, E> {
            Ok(Value::String(v))
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
            let mut items = vec![];
            while let Some(item) = seq.next_element()? {
                items.push(item);
            }
            Ok(Value::Array(items))
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
            let mut members = vec![];
            while let Some((k, v)) = map.next_entry()? {
                members.push((k, v));
            }
            Ok(Value::Object(members))
        }
    }

    impl<'de> Deserialize<'de> for Value {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            d.deserialize_any(ValueVisitor)
        }
    }
}
