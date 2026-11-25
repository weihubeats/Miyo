/**
 * 类浏览器的分组模型（业务代码 / Spring Bean / 第三方依赖 / JDK / 生成类 / 未识别）。
 *
 * 为什么放在 utils 而不是组件内：分组顺序、默认折叠态、角色分组规则都是
 * **产品语义**，需要单测覆盖（JavaClassBrowser 只负责渲染）。
 */
import type { JavaClassCategory, JavaClassInfo, JavaClassRole } from '@/types/miyoApi.d'

/** 业务代码的角色分组顺序（Controller → Service → Manager → Repository → 其他）。 */
export const ROLE_ORDER: JavaClassRole[] = [
  'controller',
  'service',
  'manager',
  'repository',
  'dto',
  'config',
  'other',
]

/** 类别分组顺序（业务优先；未知放最后，避免污染视线）。 */
export const CATEGORY_ORDER: JavaClassCategory[] = [
  'application',
  'spring_bean',
  'dependency',
  'jdk_internal',
  'generated',
  'unknown',
]

/** 默认折叠的类别：依赖 / JDK / 生成类（数量大且不是用户主要目标）。 */
const COLLAPSED_BY_DEFAULT: JavaClassCategory[] = ['dependency', 'jdk_internal', 'generated']

export interface ClassGroup {
  key: string
  category: JavaClassCategory
  /** 业务角色（仅 application / spring_bean 的角色子分组有值）。 */
  role?: JavaClassRole
  items: JavaClassInfo[]
  count: number
  collapsed: boolean
}

function categoryOf(item: JavaClassInfo): JavaClassCategory {
  return item.category ?? 'unknown'
}

/**
 * 把扁平类列表分组为可折叠树。
 *
 * @param items 行数据
 * @param favorites 收藏判定（收藏项在各组内保持原名次，由组件排序）
 * @returns 按 CATEGORY_ORDER；业务类别内再按 ROLE_ORDER 细分
 */
export function groupClasses(
  items: JavaClassInfo[],
  favorites: (className: string) => boolean = () => false,
): ClassGroup[] {
  const byCategory = new Map<JavaClassCategory, JavaClassInfo[]>()
  for (const item of items) {
    const key = categoryOf(item)
    const bucket = byCategory.get(key)
    if (bucket) bucket.push(item)
    else byCategory.set(key, [item])
  }

  const groups: ClassGroup[] = []
  for (const category of CATEGORY_ORDER) {
    const rows = byCategory.get(category)
    if (!rows || rows.length === 0) continue
    const collapse = COLLAPSED_BY_DEFAULT.includes(category)
    const hasRoles = category === 'application' || category === 'spring_bean'
    if (!hasRoles) {
      groups.push({ key: category, category, items: rows, count: rows.length, collapsed: collapse })
      continue
    }
    const byRole = new Map<JavaClassRole, JavaClassInfo[]>()
    for (const item of rows) {
      const role = item.role ?? 'other'
      const bucket = byRole.get(role)
      if (bucket) bucket.push(item)
      else byRole.set(role, [item])
    }
    for (const role of ROLE_ORDER) {
      const roleRows = byRole.get(role)
      if (!roleRows || roleRows.length === 0) continue
      groups.push({
        key: `${category}:${role}`,
        category,
        role,
        items: roleRows,
        count: roleRows.length,
        collapsed: false,
      })
    }
  }
  // 收藏优先（不改变组内顺序：组内已按后端稳定序返回）
  for (const group of groups) {
    group.items = group.items.slice().sort((a, b) => {
      const fa = favorites(a.class_name) ? 0 : 1
      const fb = favorites(b.class_name) ? 0 : 1
      return fa - fb
    })
  }
  return groups
}

/** 统计摘要行（「业务 12 · Bean 4 · 依赖 512 …」）。 */
export function statsSummary(byCategory: Record<string, number> | null | undefined): string {
  if (!byCategory) return ''
  return CATEGORY_ORDER.filter((c) => (byCategory[c] ?? 0) > 0)
    .map((c) => `${c}:${byCategory[c]}`)
    .join(' ')
}