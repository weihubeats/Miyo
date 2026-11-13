/**
 * ExportDocsDialog 单测：范围/格式选择与导出流程编排。
 * Modal Teleport 到 body，DOM 断言/点击走 document 层面；
 * Tauri dialog / opener 插件与 miyoApi、store 均以模块级 mock 替换。
 */
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import ExportDocsDialog from '@/domains/endpoint/components/docs/ExportDocsDialog.vue'
import { useLocaleStore } from '@/stores/locale'
import { makeDraft } from '@/testUtils/draftFixture'
import type { Endpoint } from '@/types/miyoApi.d'

const storeMock = vi.hoisted(() => ({
  project: { id: 'pj-1', name: '演示项目' },
}))

const apiMock = vi.hoisted(() => ({
  exportDocs: vi.fn(),
  writeTextFile: vi.fn(),
}))

const dialogMock = vi.hoisted(() => ({ open: vi.fn() }))
const openerMock = vi.hoisted(() => ({ revealItemInDir: vi.fn(async () => {}) }))
const pathMock = vi.hoisted(() => ({
  join: vi.fn(async (dir: string, name: string) => `${dir}/${name}`),
}))

vi.mock('@/stores/workspace.ts', () => ({ useWorkspaceStore: () => storeMock }))
vi.mock('@/composables/useMiyoApi.ts', () => ({ useMiyoApi: () => apiMock }))
vi.mock('@tauri-apps/plugin-dialog', () => dialogMock)
vi.mock('@tauri-apps/plugin-opener', () => openerMock)
vi.mock('@tauri-apps/api/path', () => pathMock)

function draft(): Endpoint {
  return makeDraft({ id: 'ep-1', method: 'POST', path: '/api/v1/orders' })
}

/** Modal 内容在 body 下（Teleport）。 */
function q<T extends Element>(selector: string): T | null {
  return document.body.querySelector<T>(selector)
}

function qa<T extends Element>(selector: string): T[] {
  return Array.from(document.body.querySelectorAll<T>(selector))
}

async function click(selector: string): Promise<void> {
  const el = q<HTMLElement>(selector)
  expect(el, `元素不存在：${selector}`).toBeTruthy()
  el!.click()
  await flushPromises()
}

/** 点击已获取到的元素（jsdom 原生 click + 等待渲染刷新）。 */
async function clickEl(el: Element | null | undefined): Promise<void> {
  expect(el, '目标元素不存在').toBeTruthy()
  ;(el as HTMLElement).click()
  await flushPromises()
}

function mountDialog(d: Endpoint | null = draft()) {
  const wrapper = mount(ExportDocsDialog, {
    props: { draft: d },
    attachTo: document.body,
  })
  return wrapper
}

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
  vi.clearAllMocks()
  apiMock.exportDocs.mockResolvedValue({
    content: '{"openapi":"3.0.0"}',
    suggested_name: 'openapi-演示项目-2026-08-23.json',
  })
  apiMock.writeTextFile.mockResolvedValue(undefined)
  dialogMock.open.mockResolvedValue('/Users/demo/Downloads')
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('ExportDocsDialog：渲染', () => {
  it('渲染 5 张格式卡片，默认选中 OpenAPI 且展示 JSON/YAML 子选项', async () => {
    mountDialog()
    await flushPromises()

    expect(qa('.fmt-card')).toHaveLength(5)
    expect(qa('.fmt-card')[0].className).toContain('active')
    // OpenAPI 卡片已选中 → 序列化子选项可见
    expect(q('.variant-row')).toBeTruthy()
    expect(q('.variant-row')?.textContent).toContain('YAML')
  })

  it('非 OpenAPI 卡片不显示序列化子选项', async () => {
    mountDialog()
    await flushPromises()

    await clickEl(qa('.fmt-card')[2]) // Markdown
    expect(q('.variant-row')).toBeNull()
  })
})

describe('ExportDocsDialog：导出流程', () => {
  it('默认仅当前接口：生成 → 选择目录 → 落盘 → 关闭', async () => {
    const d = draft()
    const wrapper = mountDialog(d)
    await flushPromises()

    await click('.m-foot .miyo-btn-primary')

    expect(apiMock.exportDocs).toHaveBeenCalledWith({
      projectId: 'pj-1',
      endpointId: 'ep-1',
      format: 'openapi_json',
    })
    expect(dialogMock.open).toHaveBeenCalledWith(
      expect.objectContaining({ directory: true }),
    )
    expect(apiMock.writeTextFile).toHaveBeenCalledWith(
      '/Users/demo/Downloads/openapi-演示项目-2026-08-23.json',
      '{"openapi":"3.0.0"}',
    )
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
  })

  it('OpenAPI YAML 变体正确传参', async () => {
    const wrapper = mountDialog()
    await flushPromises()

    await clickEl(qa('.variant-btn')[1]) // YAML
    await click('.m-foot .miyo-btn-primary')

    expect(apiMock.exportDocs).toHaveBeenCalledWith(
      expect.objectContaining({ format: 'openapi_yaml' }),
    )
    wrapper.unmount()
  })

  it('用户在目录选择框取消时不写盘、不关闭', async () => {
    dialogMock.open.mockResolvedValue(null)
    const wrapper = mountDialog()
    await flushPromises()

    await click('.m-foot .miyo-btn-primary')

    expect(apiMock.writeTextFile).not.toHaveBeenCalled()
    expect(wrapper.emitted('close')).toBeUndefined()
    wrapper.unmount()
  })

  it('导出失败时提示错误且不关闭', async () => {
    apiMock.exportDocs.mockRejectedValue(new Error('数据库不可用'))
    const wrapper = mountDialog()
    await flushPromises()

    await click('.m-foot .miyo-btn-primary')

    expect(apiMock.writeTextFile).not.toHaveBeenCalled()
    expect(wrapper.emitted('close')).toBeUndefined()
    wrapper.unmount()
  })
})
