/**
 * 权限目录（前端展示镜像）：唯一事实源在 Rust `miyo-plugin::permission`。
 * 这里只提供 i18n 文案键与风险等级（目录项与 Rust PERMISSIONS 一一对应，
 * 新增权限先改 Rust 再补这里与 i18n）。
 */
export type PermissionRisk = 'low' | 'medium' | 'high'

export interface PermissionDef {
  id: string
  risk: PermissionRisk
  /** i18n 键（plugins.perm.<name>）。 */
  labelKey: string
}

/** 权限目录（顺序即管理中心的展示顺序：低风险在前）。 */
export const PERMISSIONS: PermissionDef[] = [
  { id: 'std.storage', risk: 'low', labelKey: 'plugins.perm.stdStorage' },
  { id: 'std.log', risk: 'low', labelKey: 'plugins.perm.stdLog' },
  { id: 'std.notify', risk: 'low', labelKey: 'plugins.perm.stdNotify' },
  { id: 'std.events', risk: 'low', labelKey: 'plugins.perm.stdEvents' },
  { id: 'std.clipboard.write', risk: 'low', labelKey: 'plugins.perm.stdClipboard' },
  { id: 'std.env.read', risk: 'medium', labelKey: 'plugins.perm.stdEnvRead' },
  { id: 'java.debug', risk: 'high', labelKey: 'plugins.perm.javaDebug' },
]

/** 按 ID 查权限定义。 */
export function lookupPermission(id: string): PermissionDef | undefined {
  return PERMISSIONS.find((p) => p.id === id)
}

/**
 * 命令 → 权限映射（SDK commands.invoke 的白名单依据；后端另有插件启用态门禁）。
 * 未命中的命令一律拒绝——插件不能调用任意核心命令。
 */
const COMMAND_PERMISSIONS: Record<string, string> = {
  // Java 调试域（内置 java.debug 权限）
  list_java_projects: 'java.debug',
  save_java_project: 'java.debug',
  delete_java_project: 'java.debug',
  java_discover_local: 'java.debug',
  java_probe_endpoint: 'java.debug',
  java_test_connection: 'java.debug',
  java_list_classes: 'java.debug',
  java_list_methods: 'java.debug',
  java_invoke: 'java.debug',
  java_cancel_invocation: 'java.debug',
  list_java_history: 'java.debug',
  get_java_invocation: 'java.debug',
  delete_java_invocation: 'java.debug',
  clear_java_history: 'java.debug',
  // 环境读取域（std.env.read；只读）
  list_environments: 'std.env.read',
  get_projects: 'std.env.read',
}

/** 命令所需权限（未登记 = 拒绝）。 */
export function requiredPermissionFor(command: string): string | undefined {
  return COMMAND_PERMISSIONS[command]
}
