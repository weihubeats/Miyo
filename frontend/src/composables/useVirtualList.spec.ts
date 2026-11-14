/**
 * useVirtualList 单测：窗口切片 / 阈值回退 / 滚动同步 / 总高与偏移。
 * jsdom 无布局：viewportHeight 恒 0 → 可见数为 overscan 侧的兜底行为，
 * 切片边界用可控的 scrollTop 驱动验证。
 */
import { describe, expect, it } from 'vitest'
import { computed, nextTick, ref } from 'vue'
import { useVirtualList } from '@/composables/useVirtualList'

interface Row {
  id: number
}

function makeRows(n: number): Row[] {
  return Array.from({ length: n }, (_, i) => ({ id: i }))
}

describe('useVirtualList', () => {
  it('低于阈值关闭虚拟化：全量渲染、offset 0、totalHeight 0', () => {
    const container = ref<HTMLElement | null>(null)
    const items = ref(makeRows(10))
    const { virtualItems, offsetY, totalHeight, enabled } = useVirtualList(
      container,
      items,
      { itemHeight: 60, threshold: 80 },
    )
    expect(enabled.value).toBe(false)
    expect(virtualItems.value).toHaveLength(10)
    expect(virtualItems.value[9]!.index).toBe(9)
    expect(offsetY.value).toBe(0)
    expect(totalHeight.value).toBe(0)
  })

  it('达到阈值开启：切片窗口随 scrollTop 平移（含 overscan）', async () => {
    const container = ref<HTMLElement | null>(null)
    const items = ref(makeRows(200))
    const { virtualItems, offsetY, totalHeight, onScroll, enabled } = useVirtualList(
      container,
      items,
      { itemHeight: 60, overscan: 3, threshold: 80 },
    )
    expect(enabled.value).toBe(true)

    const el = document.createElement('div')
    Object.defineProperty(el, 'scrollTop', { value: 6000, writable: true })
    container.value = el
    await nextTick()

    el.dispatchEvent(new Event('scroll'))
    await nextTick()
    onScroll()
    await nextTick()

    // scrollTop 6000 / 60 = 行 100；窗口含上下 overscan 各 3
    expect(virtualItems.value[0]!.index).toBe(97)
    expect(offsetY.value).toBe(97 * 60)
    expect(totalHeight.value).toBe(200 * 60)
    // 最后一段不越界
    expect(virtualItems.value.at(-1)!.index).toBeLessThan(200)
  })

  it('items 响应式收缩：删除条目后窗口与总高同步', async () => {
    const container = ref<HTMLElement | null>(null)
    const rows = ref(makeRows(150))
    const items = computed(() => rows.value)
    const { totalHeight, virtualItems } = useVirtualList(container, items, {
      itemHeight: 60,
      threshold: 80,
    })
    expect(totalHeight.value).toBe(150 * 60)
    rows.value = makeRows(100)
    await nextTick()
    expect(totalHeight.value).toBe(100 * 60)
    expect(virtualItems.value.length).toBeGreaterThan(0)
  })
})
