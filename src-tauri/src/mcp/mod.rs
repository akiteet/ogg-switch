//! MCP (Model Context Protocol) 服务器管理模块
//!
//! 本模块负责 MCP 服务器配置的验证、同步和导入导出。
//!
//! ## 模块结构
//!
//! - `validation` - 服务器配置验证
//! - `grokbuild` - Grok Build MCP 同步和导入
//! - `toml_convert` - JSON → TOML 字段转换（Grok Build / 上游 Codex 共用格式）

mod grokbuild;
mod toml_convert;
mod validation;

// 重新导出公共 API
pub use grokbuild::{
    import_from_grokbuild, remove_server_from_grokbuild, sync_single_server_to_grokbuild,
};
