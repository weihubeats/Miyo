/**
 * JsonEditor 全屏单测：fullscreenable 开启时工具栏出现放大按钮，
 * 点击进入 fixed 全屏态，Esc 退出；未开启时无该按钮（其他调用方零感知）。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { DOMWrapper, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import JsonEditor from '@/components/ui/JsonEditor.vue'
import { useLocaleStore } from '@/stores/locale'

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

/**
 * 不 attachTo：组件渲染在 detached 容器，「出现在 document.body 下」即
 * Teleport 生效的专属证据（原位渲染不经过 body）。
 */
function mountEditor(fullscreenable: boolean) {
  return mount(JsonEditor, {
    props: { modelValue: '{"a": 1}', fullscreenable, showStatus: false },
  })
}

/**
 * 全屏态 Teleport 到 body（脱离工作区布局层叠）——wrapper.find 只搜组件
 * 根子树，全屏态断言/交互一律走 document 查询 + DOMWrapper。
 */
function editorInDoc(): HTMLElement | null {
  return document.querySelector('.json-editor')
}

describe('JsonEditor 全屏', () => {
  it('fullscreenable=false：不渲染全屏按钮（既有调用方零感知）', () => {
    const w = mountEditor(false)
    const fsBtn = w.findAll('.je-btn').find((b) => b.attributes('aria-label')?.includes('全屏'))
    expect(fsBtn).toBeUndefined()
    w.unmount()
  })

  it('点击进入全屏态：Teleport 到 body，Esc 退出回原位', async () => {
    const w = mountEditor(true)
    expect(editorInDoc()).toBeNull() // 原位渲染不经过 body

    const fsBtn = w.findAll('.je-btn').find((b) => b.attributes('aria-label')?.includes('全屏'))
    expect(fsBtn).toBeTruthy()

    await fsBtn!.trigger('click')
    // Teleport 生效：面板脱离组件根、挂到 body
    const teleported = editorInDoc()
    expect(teleported).toBeTruthy()
    expect(teleported!.classList.contains('fullscreen')).toBe(true)
    expect(teleported!.parentElement).toBe(document.body)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await w.vm.$nextTick()
    // 退出后回原位（Teleport disabled → 离开 body）
    expect(editorInDoc()).toBeNull()
    w.unmount()
    expect(editorInDoc()).toBeNull()
  })

  it('全屏中再次点击按钮退出', async () => {
    const w = mountEditor(true)
    await w.findAll('.je-btn').find((b) => b.attributes('aria-label')?.includes('全屏'))!.trigger('click')
    expect(editorInDoc()?.classList.contains('fullscreen')).toBe(true)

    // 全屏态按钮随面板在 body 下——用 DOMWrapper 交互
    const exitBtn = [...document.querySelectorAll('.je-btn')].find((b) =>
      b.getAttribute('aria-label')?.includes('退出全屏'),
    )
    expect(exitBtn).toBeTruthy()
    await new DOMWrapper(exitBtn).trigger('click')
    expect(editorInDoc()).toBeNull()
    w.unmount()
  })
})
