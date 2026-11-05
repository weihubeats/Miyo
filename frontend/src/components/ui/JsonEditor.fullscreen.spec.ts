/**
 * JsonEditor 全屏单测：fullscreenable 开启时工具栏出现放大按钮，
 * 点击进入 fixed 全屏态，Esc 退出；未开启时无该按钮（其他调用方零感知）。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import JsonEditor from './JsonEditor.vue'
import { useLocaleStore } from '../../stores/locale'

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

function mountEditor(fullscreenable: boolean) {
  return mount(JsonEditor, {
    props: { modelValue: '{"a": 1}', fullscreenable, showStatus: false },
    attachTo: document.body,
  })
}

describe('JsonEditor 全屏', () => {
  it('fullscreenable=false：不渲染全屏按钮（既有调用方零感知）', () => {
    const w = mountEditor(false)
    const fsBtn = w.findAll('.je-btn').find((b) => b.attributes('aria-label')?.includes('全屏'))
    expect(fsBtn).toBeUndefined()
    w.unmount()
  })

  it('点击进入全屏态，Esc 退出', async () => {
    const w = mountEditor(true)
    const fsBtn = w.findAll('.je-btn').find((b) => b.attributes('aria-label')?.includes('全屏'))
    expect(fsBtn).toBeTruthy()

    await fsBtn!.trigger('click')
    expect(w.find('.json-editor').classes()).toContain('fullscreen')

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await w.vm.$nextTick()
    expect(w.find('.json-editor').classes()).not.toContain('fullscreen')
    w.unmount()
  })

  it('全屏中再次点击按钮退出', async () => {
    const w = mountEditor(true)
    await w.findAll('.je-btn').find((b) => b.attributes('aria-label')?.includes('全屏'))!.trigger('click')
    expect(w.find('.json-editor').classes()).toContain('fullscreen')

    const exitBtn = w.findAll('.je-btn').find((b) => b.attributes('aria-label')?.includes('退出全屏'))
    await exitBtn!.trigger('click')
    expect(w.find('.json-editor').classes()).not.toContain('fullscreen')
    w.unmount()
  })
})
