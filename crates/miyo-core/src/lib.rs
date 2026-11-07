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
pub use render::{
    apply_global_params, apply_step_overrides, render_kv, render_spec, render_spec_typed,
};
pub use util::*;
pub use variable::{
    collect_var_refs, merge_variables, resolve_variables, resolve_variables_with, VariableMap,
};

pub type Result<T> = std::result::Result<T, AppError>;
