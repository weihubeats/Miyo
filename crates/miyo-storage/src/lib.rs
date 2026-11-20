pub mod db;
pub use db::db_err;
pub mod repository;
// 开发种子数据：仅 debug 构建参与编译（release 完全剔除）
#[cfg(debug_assertions)]
pub mod seed;
