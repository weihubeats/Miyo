import { afterEach, describe, expect, it, vi } from 'vitest'

const { mount, SortableMock } = vi.hoisted(() => {
  const mount = vi.fn()
  const obj = { mount } as { mount: typeof mount; __miyo_swap_mounted?: boolean }
  return { mount, SortableMock: obj }
})
vi.mock('sortablejs', () => ({
  default: SortableMock,
  Swap: class Swap {},
}))

import { ensureSwapMounted } from '@/utils/sortable'

afterEach(() => {
  vi.clearAllMocks()
  vi.unstubAllGlobals()
  delete SortableMock.__miyo_swap_mounted
})

describe('ensureSwapMounted', () => {
  it('重复调用时 mount 只执行一次（HMR 防重影）', () => {
    ensureSwapMounted()
    ensureSwapMounted()
    expect(mount).toHaveBeenCalledTimes(1)
    expect(SortableMock.__miyo_swap_mounted).toBe(true)
  })

  it('清除标记后允许再次挂载（标记是"本周期内已挂载"，非永久锁定）', () => {
    ensureSwapMounted()
    delete SortableMock.__miyo_swap_mounted
    ensureSwapMounted()
    expect(mount).toHaveBeenCalledTimes(2)
  })

  it('标记键写在 Sortable 对象上，跨模块重灌仍稳定', () => {
    ensureSwapMounted()
    expect('__miyo_swap_mounted' in SortableMock).toBe(true)
    expect(() => ensureSwapMounted()).not.toThrow()
    expect(mount).toHaveBeenCalledTimes(1)
  })

  it('非重复挂载的错误照常抛出（不盲目吞错）', () => {
    mount.mockImplementationOnce(() => {
      throw new Error('Swap plugin incompatible')
    })
    expect(() => ensureSwapMounted()).toThrow('Swap plugin incompatible')
  })
})