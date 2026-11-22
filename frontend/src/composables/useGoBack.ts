/**
 * 统一返回策略（全局导航规范的单点实现；规范全文见 docs/navigation.md）。
 *
 * 导航分层（本函数只承担 Back Navigation 层）：
 * - Application：DashboardNav / Brand —— 全局页面切换；
 * - Workspace：ProjectTabs / TabBar —— 工作区上下文与页签；
 * - Module：面包屑 + 返回钮 —— 模块内层级（面包屑与返回钮不堆叠同一层级入口）；
 * - Tab：页签固定 / 最近关闭 —— 见 TabBar 与 workspace store。
 *
 * 返回优先级：
 * 1. 会话内有可回退历史（`history.state.back` 由 vue-router 仅在应用内导航时
 *    写入）→ `router.back()` 原路返回来源页；
 * 2. 深链直开 / 无可回退历史 → 回 `fallback`（默认项目首页 /projects；模块内
 *    深层页面应传所属模块首页，如插件宿主传 /plugins）。
 *
 * 为什么不会退到 Miyo 之外：
 * - Tauri 单窗口 WebView 的历史栈只由本应用写入，`history.state.back` 为 null
 *   时（外部站点 → 应用、地址栏直开）一律走 fallback，绝不 `router.back()`；
 * - 刷新深层页面后 `history.state` 保留（浏览器按历史条目持久化），返回仍能
 *   原路回来源页。
 *
 * 场景 A（列表 → 详情再返回保留搜索/分页/选中）：状态由 Pinia store 承担
 * （workspace / integration / aiTest 均跨路由存活），本函数只负责路由回退；
 * 各视图滚动容器为自绘（非 window 滚动），router scrollBehavior 不适用。
 *
 * 防重复跳转：返回动作带 300ms 导航锁——双击返回钮 / 快捷键连按只生效一次，
 * 杜绝「back 两步跳过来源页」与 back↔push 死循环。
 */
import { useRouter } from 'vue-router'

/** 应用根：仪表板（/projects）。所有兜底返回的最终落点。 */
export const NAV_HOME = '/projects'

/** 导航锁窗口：该时长内的重复返回调用被忽略。 */
const BACK_LOCK_MS = 300

/** 返回策略的判定函数（导出供测试与 BackButton 复用）。 */
export function hasInAppHistory(): boolean {
  return !!window.history.state?.back
}

/**
 * 返回 hook：组件 setup 内调用，返回 goBack(fallback?)。
 *
 * @example
 * const goBack = useGoBack()
 * goBack()            // 深链兜底回 /projects
 * goBack('/plugins')  // 指定兜底目标（如插件宿主回插件中心）
 */
export function useGoBack(): (fallback?: string) => void {
  const router = useRouter()
  let lastAt = 0
  return (fallback: string = NAV_HOME): void => {
    const now = Date.now()
    if (now - lastAt < BACK_LOCK_MS) return
    lastAt = now
    if (hasInAppHistory()) router.back()
    else void router.push(fallback)
  }
}
