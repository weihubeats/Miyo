/**
 * 插件系统测试夹具：与 Rust `crates/miyo-plugin/builtins/java-debug/manifest.json`
 * 对齐的 PluginView 构造器（测试专用；真源仍在 Rust 清单，Rust 侧一致性测试防漂移）。
 */
import type { PluginManifest } from '@/types/generated/PluginManifest'
import type { PluginView } from '@/types/generated/PluginView'

export const JAVA_DEBUG_PLUGIN_ID = 'com.miyo.java-debug'

export function javaManifest(overrides?: Partial<PluginManifest>): PluginManifest {
  return {
    id: JAVA_DEBUG_PLUGIN_ID,
    name: 'Java 方法调试',
    name_key: 'pnav.javaDebug',
    description: 'Java 调试插件',
    description_key: 'plugins.javaDebugDesc',
    version: '0.1.0',
    authors: ['Miyo'],
    kind: 'ui',
    min_app_version: '0.0.60',
    sdk_version: 1,
    nav: [
      { id: 'main', label_key: 'pnav.javaDebug', icon: 'package', route: '/java-debug', order: 40 },
    ],
    permissions: [
      'std.storage',
      'std.env.read',
      'std.notify',
      'std.log',
      'std.events',
      'java.debug',
    ],
    ...overrides,
  }
}

/** 构造已安装视图（默认启用态）。 */
export function javaView(overrides?: {
  manifest?: Partial<PluginManifest>
  view?: Partial<PluginView>
}): PluginView {
  const manifest = javaManifest(overrides?.manifest)
  return {
    id: manifest.id,
    kind: 'ui',
    version: manifest.version,
    state: overrides?.view?.state ?? 'enabled',
    source: 'builtin',
    manifest,
    integrity: null,
    signed: false,
    granted_permissions: manifest.permissions,
    install_path: null,
    error: null,
    installed_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    enabled: overrides?.view?.enabled ?? true,
    ...overrides?.view,
  }
}
