/**
 * useVirtualList：固定行高虚拟滚动（无第三方依赖）。
 *
 * 只覆盖「行高恒定」的平铺长列表（请求历史 44px 行）；行高不定或分组
 * 嵌套的列表不适用（估算失准会滚动错位），继续走 content-visibility。
 *
 * 低于 threshold（默认 80 行）自动关闭虚拟化——短列表全量渲染更简单，
 * 也避免滚动容器高度测量在极端布局下引入抖动。
 *
 * 用法（宿主模板）：
 * ```html
 * <div ref="container" class="list" @scroll="onScroll">
 *   <div :style="{ height: `${totalHeight}px`, position: 'relative' }">
 *     <div :style="{ transform: `translateY(${offsetY}px)` }">
 *       <row v-for="it in virtualItems" :key="it.data.id" ... />
 *     </div>
 *   </div>
 * </div>
 * ```
 * `virtualItems` 元素带 `data`（原条目）与 `index`（原下标）。
 */
import { computed, onBeforeUnmount, onMounted, ref, type Ref } from 'vue'

export interface UseVirtualListOptions {
  /** 行高（px，恒定）。 */
  itemHeight: number
  /** 视口外上下各多渲染几行（滚动缓冲）。 */
  overscan?: number
  /** 低于该条数关闭虚拟化（全量渲染）。 */
  threshold?: number
}

export interface VirtualItem<T> {
  data: T
  /** 原列表下标。 */
  index: number
}

export function useVirtualList<T>(
  container: Ref<HTMLElement | null>,
  items: Ref<T[]>,
  options: UseVirtualListOptions,
) {
  const itemHeight = options.itemHeight
  const overscan = options.overscan ?? 6
  const threshold = options.threshold ?? 80

  const scrollTop = ref(0)
  const viewportHeight = ref(0)

  /** 低于阈值关闭：全量渲染，offsetY 恒 0。 */
  const enabled = computed(() => items.value.length >= threshold)

  const startIndex = computed(() => {
    if (!enabled.value) return 0
    return Math.max(0, Math.floor(scrollTop.value / itemHeight) - overscan)
  })
  const visibleCount = computed(() => {
    if (!enabled.value) return items.value.length
    return Math.ceil(viewportHeight.value / itemHeight) + overscan * 2
  })
  const endIndex = computed(() =>
    Math.min(items.value.length, startIndex.value + visibleCount.value),
  )

  const virtualItems = computed<VirtualItem<T>[]>(() => {
    const list: VirtualItem<T>[] = []
    for (let i = startIndex.value; i < endIndex.value; i++) {
      list.push({ data: items.value[i]!, index: i })
    }
    return list
  })

  const offsetY = computed(() =>
    enabled.value ? startIndex.value * itemHeight : 0,
  )
  const totalHeight = computed(() =>
    enabled.value ? items.value.length * itemHeight : 0,
  )

  function onScroll(): void {
    const el = container.value
    if (el) scrollTop.value = el.scrollTop
  }

  let ro: ResizeObserver | null = null
  function measure(): void {
    const el = container.value
    if (el) viewportHeight.value = el.clientHeight
  }
  onMounted(() => {
    measure()
    if (typeof ResizeObserver === 'undefined') return
    ro = new ResizeObserver(measure)
    if (container.value) ro.observe(container.value)
  })
  onBeforeUnmount(() => {
    ro?.disconnect()
    ro = null
  })

  return { virtualItems, offsetY, totalHeight, onScroll, enabled }
}
