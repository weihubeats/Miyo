pub mod curl_parser;
pub mod error;
pub mod model;
pub mod paths;
pub mod render;
pub mod util;
pub mod variable;

pub use curl_parser::{parse_curl, CurlParsed};
pub use error::{not_found, openapi_error, script_error, validation, ws_error, AppError};
pub use model::*;
pub use render::{render_kv, render_spec};
pub use util::*;
pub use variable::{merge_variables, resolve_variables, resolve_variables_with, VariableMap};

pub type Result<T> = std::result::Result<T, AppError>;
