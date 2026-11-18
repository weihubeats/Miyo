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
  { id: 'std.network', risk: 'medium', labelKey: 'plugins.perm.stdNetwork' },
  { id: 'std.files.read', risk: 'medium', labelKey: 'plugins.perm.stdFilesRead' },
  { id: 'std.files.write', risk: 'medium', labelKey: 'plugins.perm.stdFilesWrite' },
  { id: 'runtime.process.start', risk: 'high', labelKey: 'plugins.perm.runtimeProcessStart' },
  { id: 'runtime.process.stop', risk: 'medium', labelKey: 'plugins.perm.runtimeProcessStop' },
  { id: 'java.process.discover', risk: 'medium', labelKey: 'plugins.perm.javaProcessDiscover' },
  { id: 'java.runtime.attach', risk: 'high', labelKey: 'plugins.perm.javaRuntimeAttach' },
  { id: 'java.class.inspect', risk: 'high', labelKey: 'plugins.perm.javaClassInspect' },
  { id: 'java.method.invoke', risk: 'high', labelKey: 'plugins.perm.javaMethodInvoke' },
  { id: 'java.debug', risk: 'high', labelKey: 'plugins.perm.javaDebug' },
  { id: 'database.connect', risk: 'high', labelKey: 'plugins.perm.databaseConnect' },
  { id: 'database.execute', risk: 'high', labelKey: 'plugins.perm.databaseExecute' },
]

/** java 聚合权限（java.debug）展开的子权限（与 Rust JAVA_SUB_PERMISSIONS 镜像）。 */
export const JAVA_SUB_PERMISSIONS = [
  'java.class.inspect',
  'java.method.invoke',
  'java.process.discover',
  'java.runtime.attach',
] as const

/**
 * 展开用户批准快照：`java.debug`（0033 之前的存量行）等价于全部 java.*
 * 子权限——与后端 `granted_effective` 同口径。前端 SDK 判权用展开集。
 */
export function effectiveGranted(view: { granted_permissions: string[] }): Set<string> {
  const granted = new Set(view.granted_permissions)
  if (granted.has('java.debug')) {
    for (const sub of JAVA_SUB_PERMISSIONS) granted.add(sub)
  }
  return granted
}

/** 按 ID 查权限定义。 */
export function lookupPermission(id: string): PermissionDef | undefined {
  return PERMISSIONS.find((p) => p.id === id)
}

/**
 * 命令 → 权限映射（SDK commands.invoke 的白名单依据；后端另有插件门禁 +
 * granted 快照逐命令校验，两侧同口径）。
 * 未命中的命令一律拒绝——插件不能调用任意核心命令。
 * 数据管理类命令（项目 / 历史 CRUD）仅需插件启用态，不登记。
 */
const COMMAND_PERMISSIONS: Record<string, string> = {
  // Java 进程发现（含 Runtime 模式的端点探测 / 连接测试）
  java_discover_local: 'java.process.discover',
  java_probe_endpoint: 'java.process.discover',
  java_test_connection: 'java.process.discover',
  // Java 类 / 方法 / Bean 探测
  java_list_classes: 'java.class.inspect',
  java_list_methods: 'java.class.inspect',
  // Java 方法执行（最高危）
  java_invoke: 'java.method.invoke',
  // 在途调用取消（触及 Runtime 会话）
  java_cancel_invocation: 'java.runtime.attach',
  // v2 Attach 命令族
  java_discover_processes: 'java.process.discover',
  java_probe_pid: 'java.process.discover',
  java_attach_process: 'java.runtime.attach',
  java_detach_process: 'java.runtime.attach',
  java_unload_agent: 'java.runtime.attach',
  java_attach_sessions: 'java.runtime.attach',
  java_agent_status: 'java.runtime.attach',
  java_agent_spring_status: 'java.class.inspect',
  java_agent_classes: 'java.class.inspect',
  java_agent_class_detail: 'java.class.inspect',
  java_agent_methods: 'java.class.inspect',
  java_agent_beans: 'java.class.inspect',
  java_agent_invoke: 'java.method.invoke',
  java_agent_cancel: 'java.runtime.attach',
  // Sidecar 生命周期（常驻型插件进程；宿主视图 plugin_sidecar_status 不经 SDK）
  plugin_sidecar_start: 'runtime.process.start',
  plugin_sidecar_stop: 'runtime.process.stop',
  plugin_sidecar_request: 'runtime.process.start',
  // 环境读取域（std.env.read；只读）
  list_environments: 'std.env.read',
  get_projects: 'std.env.read',
}

/** 命令所需权限（未登记 = 拒绝）。 */
export function requiredPermissionFor(command: string): string | undefined {
  return COMMAND_PERMISSIONS[command]
}
