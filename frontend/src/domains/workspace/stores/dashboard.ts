/**
 * dashboard.ts：仪表板（首页）数据快照。
 *
 * 存在意义：`/projects` 与 `/realtime` 是两条独立路由，来回切换时
 * ProjectList 会卸载后重新挂载，视图内的 ref 全部重建。若没有这层缓存，
 * 每次返回首页都要先渲染骨架屏再等 IPC 回来，视觉上「闪一下」。
 *
 * 快照挂在 pinia 上（应用级单例），返回首页时先复用上一份数据、
 * 再静默刷新；项目 / 团队 / 统计的写入统一经 setter 落一份副本，
 * 避免视图侧的修改直接改到缓存对象上。
 */
import { ref } from 'vue'
import { defineStore } from 'pinia'
import type { AggregatedProjectStats } from '@/utils/projectStats'
import type { Project, Team } from '@/types/miyoApi.d'

/** 统计的空值骨架（首次进入尚未加载时用）。 */
export function emptyProjectStats(): AggregatedProjectStats {
  return {
    counts: {},
    scenarioCounts: {},
    requestCounts: {},
    okCounts: {},
    failCounts: {},
    latest: {},
  }
}

export const useDashboardStore = defineStore('dashboard', () => {
  /** 最近一次成功拉到的项目列表（null = 从未加载成功）。 */
  const projects = ref<Project[] | null>(null)
  const teams = ref<Team[] | null>(null)
  const stats = ref<AggregatedProjectStats | null>(null)

  /** 是否有可用于首屏渲染的快照（决定要不要走骨架屏）。 */
  function hasSnapshot(): boolean {
    return projects.value !== null || stats.value !== null
  }

  function setProjects(list: Project[]): void {
    projects.value = list
  }

  function setTeams(list: Team[]): void {
    teams.value = list
  }

  function setStats(next: AggregatedProjectStats): void {
    stats.value = next
  }

  return { projects, teams, stats, hasSnapshot, setProjects, setTeams, setStats }
})