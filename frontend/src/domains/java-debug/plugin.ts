/**
 * Java 方法调试 —— 内置插件入口（PluginFactory）。
 *
 * 激活（插件已安装且启用后由 PluginManager 调用）：
 * 1. 按 manifest.nav 注册导航（DashboardNav / WorkspaceView 工具组渲染）；
 * 2. 注册 /java-debug 路由（懒加载 JavaDebugView，独立 chunk）。
 *
 * 停用：导航 / 路由由 PluginManager 统一回收；这里负责插件自有运行时资源——
 * javaDebug store 的心跳定时器与在途状态（dispose 动作）。
 *
 * 后端对应物：miyo-plugin/builtins/java-debug/manifest.json（唯一事实源）；
 * 14 个 java_* IPC 命令在 Rust 侧按插件状态门禁（未安装 / 停用即拒绝）。
 */
import type { PluginFactory } from '@/plugins/types'

export const JAVA_DEBUG_ROUTE = '/java-debug'

const plugin: PluginFactory = (ctx) => {
  // 导航：manifest 声明即注册（运行时可再增删，Java 插件只有单一入口项）。
  for (const item of ctx.manifest.nav ?? []) {
    ctx.ui.nav.register(item)
  }
  // 路由：懒加载工作台视图（chunk 含三栏 SplitPanes + 全部子组件）。
  ctx.ui.route.register({
    path: JAVA_DEBUG_ROUTE,
    name: 'plugin-java-debug-main',
    load: () => import('./views/JavaDebugView.vue'),
  })

  ctx.logger.debug(`java-debug plugin activated v${ctx.manifest.version}`)

  return {
    async deactivate() {
      // 释放插件运行时：连接心跳 / 在途状态 / 发现缓存（模块动态 import——
      // deactivate 时该 store 通常已存在；不存在则无资源可释放）。
      try {
        const { useJavaDebugStore } = await import('./stores/javaDebug')
        useJavaDebugStore().dispose()
      } catch (err) {
        ctx.logger.warn(`javaDebug store dispose failed: ${String(err)}`)
      }
    },
  }
}

export default plugin
