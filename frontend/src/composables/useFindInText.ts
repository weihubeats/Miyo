/**
 * useFindInText：响应查看器与请求 Body 编辑器共用的「文本内查找」状态机。
 *
 * 两处原本各自手写同一套管线（findOpen / query / 160ms 防抖 searchQuery /
 * activeMatch / 总数越界回缩 / next / prev / close 清零），修 bug 要改两份
 * 且已出现口径分叉。此 composable 收口状态机；**计数来源由调用方提供**
 *（树视图上报 / 行视图统计 / CodeMirror 偏移定位各不相同），`onClose`
 * 供调用方清零各自的外部计数。
 */
import { onMounted, onUnmounted, ref, watch, type Ref } from 'vue'

export interface UseFindInTextOptions {
  /** closeFind 时额外清零的外部计数（如树视图 treeTotal / 编辑器 jsonTotal）。 */
  onClose?: () => void
  /** 输入防抖毫秒数（默认 160：大文本上匹配计算是全量的）。 */
  debounceMs?: number
}

/** 大小写不敏感的出现次数统计（0 长度词不计数）。 */
export function countOccurrences(text: string, q: string): number {
  if (!text || !q) return 0
  const ql = q.toLowerCase()
  const lower = text.toLowerCase()
  let n = 0
  let from = 0
  for (;;) {
    const idx = lower.indexOf(ql, from)
    if (idx === -1) break
    n += 1
    from = idx + ql.length
  }
  return n
}

export function useFindInText(total: Ref<number>, options: UseFindInTextOptions = {}) {
  const findOpen = ref(false)
  const query = ref('')
  const activeMatch = ref(0)

  /**
   * 防抖后的查找词：匹配 / 高亮 / 计数全部基于它。输入框本身（FindBar
   * v-model）仍用即时 query 保持跟手。空词立即生效，保证清空搜索时高亮
   * 无残留。
   */
  const searchQuery = ref('')
  let searchTimer: ReturnType<typeof setTimeout> | undefined
  watch(query, (q) => {
    if (searchTimer) clearTimeout(searchTimer)
    if (!q) {
      searchQuery.value = ''
      return
    }
    searchTimer = setTimeout(() => {
      searchQuery.value = q
    }, options.debounceMs ?? 160)
  })

  watch(query, () => {
    activeMatch.value = 0
  })

  // 宿主卸载时丢弃未触发的防抖（回调里写已卸载组件的 ref 无害，但清掉更干净）。
  onUnmounted(() => {
    if (searchTimer) clearTimeout(searchTimer)
  })

  // 总数变化时把激活序号收回界内（结果变少后停留在旧序号会指向不存在的匹配）。
  // 延后到 mounted 注册：total 的 getter 普遍引用宿主在本次调用之后才解构出的
  // searchQuery 等状态（counting computed 与状态机互为前后件），setup 期立即
  // 求值会踩暂时性死区（BodyPanel 首挂即抛）。
  onMounted(() => {
    watch(total, (t) => {
      if (t === 0) activeMatch.value = 0
      else if (activeMatch.value >= t) activeMatch.value = t - 1
    })
  })

  function nextMatch(): void {
    if (!total.value) return
    activeMatch.value = (activeMatch.value + 1) % total.value
  }

  function prevMatch(): void {
    if (!total.value) return
    activeMatch.value = (activeMatch.value - 1 + total.value) % total.value
  }

  function closeFind(): void {
    findOpen.value = false
    query.value = ''
    activeMatch.value = 0
    options.onClose?.()
  }

  function toggleFind(): void {
    if (findOpen.value) closeFind()
    else findOpen.value = true
  }

  return {
    findOpen,
    query,
    searchQuery,
    activeMatch,
    nextMatch,
    prevMatch,
    closeFind,
    toggleFind,
  }
}
