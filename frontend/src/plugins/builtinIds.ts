/**
 * 内置插件 ID 常量（前端侧）：与 Rust `miyo_plugin::JAVA_DEBUG_PLUGIN_ID` 一致。
 * 单独成文件避免 builtinModules / manager 直接 import Rust crate 路径。
 */
export const JAVA_DEBUG_PLUGIN_ID = 'com.miyo.java-debug'
