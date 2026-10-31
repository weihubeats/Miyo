import { describe, expect, it, vi } from 'vitest'
import { computed, defineComponent, ref, type Ref } from 'vue'
import { mount } from '@vue/test-utils'
import { countOccurrences, useFindInText } from './useFindInText'

describe('countOccurrences', () => {
  it('大小写不敏感统计，空词/空文本为 0', () => {
    expect(countOccurrences('aXbXc', 'x')).toBe(2)
    expect(countOccurrences('abc', '')).toBe(0)
    expect(countOccurrences('', 'a')).toBe(0)
  })
})

type FindApi = {
  findOpen: boolean
  query: string
  searchQuery: string
  activeMatch: number
  nextMatch: () => void
  prevMatch: () => void
  closeFind: () => void
  toggleFind: () => void
}

/** 宿主组件：composable 依赖组件生命周期（mounted 注册 total 监听）。 */
function makeHost(totalSource: Ref<number>, onClose?: () => void) {
  return defineComponent({
    setup(_, { expose }) {
      const total = computed(() => totalSource.value)
      const api = useFindInText(total, { onClose })
      expose(api as unknown as FindApi)
      return () => null
    },
  })
}

const flush = () => new Promise((r) => setTimeout(r, 180))

describe('useFindInText', () => {
  it('防抖后同步 searchQuery；清空立即生效', async () => {
    const totalSource = ref(0)
    const wrapper = mount(makeHost(totalSource))
    const api = wrapper.vm as unknown as FindApi
    api.query = 'abc'
    expect(api.searchQuery).toBe('')
    await flush()
    expect(api.searchQuery).toBe('abc')
    api.query = ''
    await Promise.resolve()
    expect(api.searchQuery).toBe('')
    wrapper.unmount()
  })

  it('next/prev 循环导航且越界回缩；总数为 0 时不动作', async () => {
    const totalSource = ref(3)
    const wrapper = mount(makeHost(totalSource))
    const api = wrapper.vm as unknown as FindApi
    api.nextMatch()
    expect(api.activeMatch).toBe(1)
    api.nextMatch()
    api.nextMatch()
    expect(api.activeMatch).toBe(0)
    api.prevMatch()
    expect(api.activeMatch).toBe(2)
    // 总数缩水：激活序号收回界内（监听在 mounted 后注册）
    await flush()
    totalSource.value = 1
    await flush()
    expect(api.activeMatch).toBe(0)
    // 总数为 0：导航不动作
    totalSource.value = 0
    await flush()
    api.nextMatch()
    expect(api.activeMatch).toBe(0)
    wrapper.unmount()
  })

  it('closeFind 清零并回调外部计数清理；toggle 往返', async () => {
    const totalSource = ref(2)
    const onClose = vi.fn()
    const wrapper = mount(makeHost(totalSource, onClose))
    const api = wrapper.vm as unknown as FindApi
    api.toggleFind()
    expect(api.findOpen).toBe(true)
    api.query = 'x'
    api.activeMatch = 1
    api.toggleFind()
    expect(api.findOpen).toBe(false)
    expect(api.query).toBe('')
    expect(api.activeMatch).toBe(0)
    expect(onClose).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })
})
