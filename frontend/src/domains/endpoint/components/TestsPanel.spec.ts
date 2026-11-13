/**
 * TestsPanel 单测：断言 JSON 编辑器 Tab 缩进（焦点不跳出）。
 */
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { reactive } from 'vue'
import TestsPanel from '@/domains/endpoint/components/TestsPanel.vue'
import { useLocaleStore } from '@/stores/locale'
import { makeDraft } from '@/testUtils/draftFixture'

describe('TestsPanel：断言 JSON 的 Tab 缩进', () => {
  it('按 Tab 插入 2 空格并同步 v-model（焦点不跳出）', async () => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    const draft = reactive(makeDraft())
    const wrapper = mount(TestsPanel, { props: { draft, url: 'http://localhost/x' } })
    const ta = wrapper.find('textarea.tp-input')
    expect(ta.exists()).toBe(true)
    await ta.setValue('{\n}')
    const el = ta.element as HTMLTextAreaElement
    el.selectionStart = el.selectionEnd = 2
    await ta.trigger('keydown', { key: 'Tab' })
    expect(el.value).toBe('{\n  }')
    expect(el.selectionStart).toBe(4)
  })
})
