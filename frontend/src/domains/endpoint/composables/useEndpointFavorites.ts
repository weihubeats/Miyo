/**
 * useEndpointFavorites：接口收藏（纯视图偏好）。
 *
 * - 刻意不入库：不改 Endpoint 数据结构与后端契约，收藏只存前端
 *   localStorage（按项目隔离：`miyo.endpoint-favorites.{projectId}`）；
 * - 模块级单例状态：EndpointTree 是递归组件，星标按钮在任意层实例、
 *   收藏分组渲染在根实例，两侧必须共享同一份读写；
 * - 读取时与当前项目 endpoints 求交集：已删除的接口自动从分组消失，
 *   id 留在存储里不清理（换项目/重建后同名 id 恢复可见，成本为零）。
 */
import { computed, ref } from 'vue'
import { useWorkspaceStore } from '@/stores/workspace'
import type { Endpoint } from '@/types/miyoApi.d'

const FAV_PREFIX = 'miyo.endpoint-favorites.'

const favoritesByProject = ref<Record<string, string[]>>(loadAll())

function loadAll(): Record<string, string[]> {
  const out: Record<string, string[]> = {}
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i)
      if (!key?.startsWith(FAV_PREFIX)) continue
      const arr: unknown = JSON.parse(localStorage.getItem(key) ?? '[]')
      if (Array.isArray(arr)) {
        out[key.slice(FAV_PREFIX.length)] = arr.filter((v): v is string => typeof v === 'string')
      }
    }
  } catch {
    // 单条损坏不拖垮整体：解析失败保持已解析部分
  }
  return out
}

export function useEndpointFavorites() {
  const store = useWorkspaceStore()
  const projectId = computed(() => store.project?.id ?? null)

  /** 当前项目收藏 id 集合（行级星标态判断）。 */
  const favoriteIds = computed<Set<string>>(() => {
    const pid = projectId.value
    return new Set(pid ? (favoritesByProject.value[pid] ?? []) : [])
  })

  function isFavorite(id: string): boolean {
    return favoriteIds.value.has(id)
  }

  /** 切换收藏：新收藏插到最前（分组内按收藏时间倒序）。 */
  function toggleFavorite(id: string): void {
    const pid = projectId.value
    if (!pid) return
    const cur = favoritesByProject.value[pid] ?? []
    const next = cur.includes(id) ? cur.filter((x) => x !== id) : [id, ...cur]
    favoritesByProject.value = { ...favoritesByProject.value, [pid]: next }
    try {
      localStorage.setItem(FAV_PREFIX + pid, JSON.stringify(next))
    } catch {
      // 配额满不影响主流程（与 recents 同策略）
    }
  }

  /** 收藏分组数据：按收藏序排列、过滤已删除接口。 */
  const favoriteEndpoints = computed<Endpoint[]>(() => {
    const pid = projectId.value
    if (!pid) return []
    const ids = favoritesByProject.value[pid] ?? []
    const out: Endpoint[] = []
    for (const id of ids) {
      const ep = store.endpoints.find((e) => e.id === id)
      if (ep) out.push(ep)
    }
    return out
  })

  return { favoriteIds, isFavorite, toggleFavorite, favoriteEndpoints }
}

/** 测试复位：模块级收藏状态跨用例存活，spec 显式调用避免串现场。 */
export function __resetEndpointFavorites(): void {
  favoritesByProject.value = {}
}
