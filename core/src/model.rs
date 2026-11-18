//! Plain data model shared by the scanner, the OpenAPI writer and the Apifox client.
//! No host calls here: everything in this crate is pure and unit-testable.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    String,
    Integer,
    Long,
    Number,
    Boolean,
    /// `string` with `format: binary`
    Binary,
    Array(Box<Ty>),
    Object(Vec<Field>),
    /// `$ref` to `components.schemas`
    Ref(String),
    /// A Java enum: `{ "type": "string", "enum": [...] }`.
    Enum(Vec<String>),
    /// `{}` — a type we know nothing about
    Any,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: Ty,
    pub required: bool,
    pub description: String,
    /// Bean Validation constraints, carried to the JSON Schema.
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub min_len: Option<u64>,
    pub max_len: Option<u64>,
    pub pattern: Option<String>,
    pub format: Option<String>,
    /// `example` from `@Schema` / `@ApiModelProperty`; empty = none.
    pub example: String,
}

impl Field {
    /// A field without constraints; `spring` fills them from annotations.
    pub fn new(name: impl Into<String>, ty: Ty, required: bool, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ty,
            required,
            description: description.into(),
            min: None,
            max: None,
            min_len: None,
            max_len: None,
            pattern: None,
            format: None,
            example: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamIn {
    Query,
    Path,
    Header,
    Cookie,
}

impl ParamIn {
    pub fn as_str(self) -> &'static str {
        match self {
            ParamIn::Query => "query",
            ParamIn::Path => "path",
            ParamIn::Header => "header",
            ParamIn::Cookie => "cookie",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub location: ParamIn,
    pub required: bool,
    pub description: String,
    pub ty: Ty,
    pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    /// HTTP status code as text (`"200"`, `"401"` ...).
    pub code: String,
    pub description: String,
    /// Media type of this response body.
    pub media: String,
    /// `None` = no body (description only).
    pub ty: Option<Ty>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    pub method: String,
    pub path: String,
    pub tag: String,
    pub summary: String,
    pub description: String,
    pub operation_id: String,
    pub params: Vec<Param>,
    /// media type -> schema
    pub body: Option<(String, Ty)>,
    /// Declared `@ApiResponse` entries plus the default success response.
    pub responses: Vec<Response>,
    pub deprecated: bool,
    /// 1-based source line of the handler, for IDE navigation.
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiDoc {
    pub title: String,
    pub version: String,
    pub endpoints: Vec<Endpoint>,
    pub schemas: BTreeMap<String, Ty>,
}

impl ApiDoc {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            version: "1.0.0".into(),
            endpoints: Vec::new(),
            schemas: BTreeMap::new(),
        }
    }
}