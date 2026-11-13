import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import CodeExportDialog from '@/domains/endpoint/components/CodeExportDialog.vue'
import { useLocaleStore } from '@/stores/locale'
import { collectErrors } from '@/testUtils/componentTest'
import { makeCodegenInputs } from '@/testUtils/draftFixture'

const codegenRender = vi.fn()

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => ({
    pending: { value: false },
    codegenRender,
  }),
}))

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
})

function dialogBody(): Element | null {
  return document.body.querySelector('.m-dialog')
}

describe('CodeExportDialog：导出接口代码', () => {
  afterEach(() => {
    codegenRender.mockReset()
    document.body.innerHTML = ''
  })

  it('打开即自动生成并可关闭', async () => {
    codegenRender.mockResolvedValue('curl -X GET "https://api.example.com"')
    const wrapper = mount(CodeExportDialog, {
      props: { ...makeCodegenInputs() },
      attachTo: document.body,
    })
    await flushPromises()
    expect(codegenRender).toHaveBeenCalledTimes(1)
    expect(dialogBody()?.querySelector('.m-title')?.textContent).toContain('导出接口代码')
    expect(dialogBody()?.querySelector('.cp-preview')?.textContent).toContain('curl')

    // Modal 通过 Teleport 渲染到 body，需从 document 层面点击关闭按钮。
    const closeBtn = dialogBody()?.querySelector<HTMLButtonElement>('.m-foot .miyo-btn-primary')
    closeBtn?.click()
    await flushPromises()
    expect(wrapper.emitted('close')).toBeTruthy()
    wrapper.unmount()
  })

  it('生成未返回时关闭弹窗（立即卸载）不产生未处理错误', async () => {
    const collector = collectErrors()
    let resolve!: (v: string) => void
    codegenRender.mockImplementation(() => new Promise((r) => (resolve = r)))

    const wrapper = mount(CodeExportDialog, {
      props: { ...makeCodegenInputs() },
      attachTo: document.body,
    })
    await flushPromises()
    wrapper.unmount()
    resolve('late')
    await flushPromises()

    expect(collector.errors).toEqual([])
    collector.restore()
  })
})