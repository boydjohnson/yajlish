//! Validation errors.

/// Why a record (or the document around it) does not match the schemas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    /// JSON Pointer into the record, e.g. `/geometry/coordinates/0`. Empty
    /// for the record itself.
    pub instance_path: String,
    /// The schema location of the failed keyword, e.g.
    /// `point.schema.json#/properties/geometry/type`.
    pub schema_path: String,
    /// The keyword that failed: `type`, `required`, `maximum`, ... Two
    /// keywords are not JSON Schema keywords: `match` (no schema, or more
    /// than one with `match = unique`, matched the record) and `build` (a
    /// valid value has no Rust representation).
    pub keyword: &'static str,
    /// Byte offset in the stream of the event where the error was found.
    pub offset: Option<u64>,
    /// A human readable description.
    pub detail: String,
    /// For `match`, `anyOf`, `oneOf`: the error from each alternative.
    pub causes: Vec<SchemaError>,
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} at '{}' ({}): {}",
            self.keyword, self.instance_path, self.schema_path, self.detail
        )?;
        if let Some(offset) = self.offset {
            write!(f, " (byte {})", offset)?;
        }
        for cause in &self.causes {
            write!(f, "\n  - {}", cause.to_string().replace('\n', "\n    "))?;
        }
        Ok(())
    }
}

impl std::error::Error for SchemaError {}

/// One step of an instance path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Seg {
    Key(String),
    Index(u64),
}

/// A `SchemaError` under construction: the instance path is collected from
/// the innermost value outwards while the error propagates up.
#[derive(Debug, Clone)]
pub(crate) struct Failure {
    /// Innermost first.
    pub(crate) segs: Vec<Seg>,
    pub(crate) schema_path: String,
    pub(crate) keyword: &'static str,
    pub(crate) offset: u64,
    pub(crate) detail: String,
    pub(crate) causes: Vec<SchemaError>,
}

impl Failure {
    pub(crate) fn new(
        node_path: &str,
        keyword: &'static str,
        offset: u64,
        detail: impl Into<String>,
    ) -> Box<Self> {
        Box::new(Failure {
            segs: vec![],
            schema_path: if keyword.is_empty() {
                node_path.to_owned()
            } else {
                format!("{}/{}", node_path, keyword)
            },
            keyword,
            offset,
            detail: detail.into(),
            causes: vec![],
        })
    }

    pub(crate) fn at(mut self: Box<Self>, seg: Seg) -> Box<Self> {
        self.segs.push(seg);
        self
    }

    pub(crate) fn into_error(self) -> SchemaError {
        let mut instance_path = String::new();
        for seg in self.segs.iter().rev() {
            instance_path.push('/');
            match seg {
                Seg::Key(k) => instance_path.push_str(&k.replace('~', "~0").replace('/', "~1")),
                Seg::Index(i) => instance_path.push_str(&i.to_string()),
            }
        }
        let mut causes = self.causes;
        for cause in &mut causes {
            cause.prefix_path(&instance_path);
        }
        SchemaError {
            instance_path,
            schema_path: self.schema_path,
            keyword: self.keyword,
            offset: Some(self.offset),
            detail: self.detail,
            causes,
        }
    }
}

impl SchemaError {
    /// Causes are recorded relative to the value that had alternatives;
    /// make them relative to the record like their parent.
    fn prefix_path(&mut self, prefix: &str) {
        self.instance_path.insert_str(0, prefix);
        for cause in &mut self.causes {
            cause.prefix_path(prefix);
        }
    }
}
