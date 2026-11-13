/**
 * endpointTreeExpanded：目录树展开态的共享与持久化。
 *
 * EndpointTree 是递归组件——每个子树是独立实例，展开状态必须全树同源，
 * 否则「父实例折叠、子实例仍认为展开」。这里用模块级 ref 单源 + 按项目
 * 写 localStorage（键 miyo.tree-expanded.{projectId}），重启后恢复上次
 * 展开的文件夹（审计 B8：原先重启全折叠，大目录每次都要重新展开）。
 */
import { ref, watch } from 'vue'

const EXPANDED_PREFIX = 'miyo.tree-expanded.'

const expanded = ref(new Set<string>())
let currentProjectId: string | null = null
let persistWatchers = 0

// 测试隔离出口：模块级状态跨用例存活，spec 显式复位（globalThis 钩子对产物无影响）。
;(globalThis as { __resetTreeExpanded?: () => void }).__resetTreeExpanded = () => {
  currentProjectId = null
  expanded.value = new Set()
}

/**
 * 切换项目时加载该项目的展开集。
 * 同项目重复调用（递归子实例挂载）直接跳过——刚展开还未落盘的运行时状态
 * 会被存储快照覆盖（展开→子实例挂载→load 回读旧值→「折叠失败」）。
 */
function loadExpanded(projectId: string | null): void {
  if (projectId === currentProjectId) return
  currentProjectId = projectId
  const next = new Set<string>()
  if (projectId) {
    try {
      const raw = localStorage.getItem(EXPANDED_PREFIX + projectId)
      if (raw) {
        for (const id of JSON.parse(raw) as unknown[]) {
          if (typeof id === 'string') next.add(id)
        }
      }
    } catch {
      // 损坏即全折叠，行为与旧版一致
    }
  }
  expanded.value = next
}

export function useTreeExpanded(): {
  expanded: typeof expanded
  loadExpanded: typeof loadExpanded
} {
  // 首个调用者注册持久化 watcher（deep：组件侧直接 add/delete Set，
  // deep watch 兜底捕获变更）；后续递归实例不再重复注册。
  if (persistWatchers === 0) {
    persistWatchers++
    watch(
      expanded,
      () => {
        if (!currentProjectId) return
        try {
          localStorage.setItem(
            EXPANDED_PREFIX + currentProjectId,
            JSON.stringify([...expanded.value]),
          )
        } catch {
          // 配额满等写失败静默：仅损失跨重启恢复
        }
      },
      { deep: true },
    )
  }
  return { expanded, loadExpanded }
}
