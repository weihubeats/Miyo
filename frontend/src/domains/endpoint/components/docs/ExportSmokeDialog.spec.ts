/**
 * ExportSmokeDialog 单测：范围选择与冒烟文档导出流程编排。
 * Modal Teleport 到 body，DOM 断言/点击走 document 层面；
 * Tauri dialog / opener 插件与 miyoApi、store 均以模块级 mock 替换。
 */
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import ExportSmokeDialog from '@/domains/endpoint/components/docs/ExportSmokeDialog.vue'
import { useLocaleStore } from '@/stores/locale'
import { makeDraft } from '@/testUtils/draftFixture'
import type { Endpoint } from '@/types/miyoApi.d'

const storeMock = vi.hoisted(() => ({
  project: { id: 'pj-1', name: '演示项目' },
  caseRunMeta: new Map<string, { status: number; durationMs: number }>([
    ['case-1', { status: 200, durationMs: 320 }],
  ]),
}))

const apiMock = vi.hoisted(() => ({
  exportSmokeDocs: vi.fn(),
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

async function click(selector: string): Promise<void> {
  const el = q<HTMLElement>(selector)
  expect(el, `元素不存在：${selector}`).toBeTruthy()
  el!.click()
  await flushPromises()
}

function mountDialog(d: Endpoint | null = draft()) {
  const wrapper = mount(ExportSmokeDialog, {
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
  apiMock.exportSmokeDocs.mockResolvedValue({
    content: '# 演示项目 冒烟测试文档',
    suggested_name: 'smoke-演示项目-2026-08-23.md',
  })
  apiMock.writeTextFile.mockResolvedValue(undefined)
  dialogMock.open.mockResolvedValue('/Users/demo/Downloads')
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('ExportSmokeDialog：渲染', () => {
  it('默认选中当前接口范围', async () => {
    mountDialog()
    await flushPromises()

    expect(q('.scope-card')?.className).toContain('active')
    expect(q('.fmt-card')?.textContent).toContain('Markdown (.md)')
  })
})

describe('ExportSmokeDialog：导出流程', () => {
  it('默认仅当前接口：生成 → 选择目录 → 落盘 → 关闭', async () => {
    const d = draft()
    const wrapper = mountDialog(d)
    await flushPromises()

    await click('.m-foot .miyo-btn-primary')

    expect(apiMock.exportSmokeDocs).toHaveBeenCalledWith({
      projectId: 'pj-1',
      endpointId: 'ep-1',
      includeResults: false,
    })
    expect(dialogMock.open).toHaveBeenCalledWith(
      expect.objectContaining({ directory: true }),
    )
    expect(apiMock.writeTextFile).toHaveBeenCalledWith(
      '/Users/demo/Downloads/smoke-演示项目-2026-08-23.md',
      '# 演示项目 冒烟测试文档',
    )
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
  })

  it('切到整个项目范围时 endpointId 传 null', async () => {
    const wrapper = mountDialog()
    await flushPromises()

    await click('.scope-grid .scope-card:last-child')
    await click('.m-foot .miyo-btn-primary')

    expect(apiMock.exportSmokeDocs).toHaveBeenCalledWith({
      projectId: 'pj-1',
      endpointId: null,
      includeResults: false,
    })
    wrapper.unmount()
  })

  it('勾选导出运行结果时 includeResults 传 true', async () => {
    const wrapper = mountDialog()
    await flushPromises()

    const check = q<HTMLInputElement>('#smoke-include-results')
    expect(check).toBeTruthy()
    check!.checked = true
    check!.dispatchEvent(new Event('change'))
    await flushPromises()

    await click('.m-foot .miyo-btn-primary')

    expect(apiMock.exportSmokeDocs).toHaveBeenCalledWith({
      projectId: 'pj-1',
      endpointId: 'ep-1',
      includeResults: true,
      runResults: { 'case-1': { status: 200, durationMs: 320 } },
    })
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
    apiMock.exportSmokeDocs.mockRejectedValue(new Error('数据库不可用'))
    const wrapper = mountDialog()
    await flushPromises()

    await click('.m-foot .miyo-btn-primary')

    expect(apiMock.writeTextFile).not.toHaveBeenCalled()
    expect(wrapper.emitted('close')).toBeUndefined()
    wrapper.unmount()
  })
})
