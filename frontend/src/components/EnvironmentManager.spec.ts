/**
 * EnvironmentManager 新建环境回归：
 * - 新建环境后未做任何编辑，直接关闭不得弹出「未保存修改」确认——
 *   创建本身不算修改（历史 bug：点击新建后关闭必弹确认）；
 *   未编辑的新环境从未落库，随关闭静默丢弃。
 * - 一旦实际编辑（如改名称），关闭必须弹出确认条且弹窗保持打开。
 * - 编辑后保存，关闭同样不弹确认（保存即清脏）。
 * - 左侧行内重命名：新建即进入编辑（全选默认名），回车提交、Esc 取消、双击已有行改名。
 * - 未保存三选确认：切换时弹居中确认，「保存并继续」落库后执行切换、「放弃修改」
 *   直接切换、「继续编辑」留在原环境。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import EnvironmentManager from './EnvironmentManager.vue'
import { useLocaleStore } from '../stores/locale'
import type { Environment } from '../types/miyoApi'

const ENVS: Environment[] = [
  {
    id: 'e-dev',
    project_id: 'p1',
    name: '开发环境',
    base_url: 'https://api.dev.example.com',
    variables: [],
    created_at: '',
    updated_at: '',
  },
]

const updateEnvironment = vi.fn(async (env: Environment) => ({ ...env }))

vi.mock('../stores/workspace', () => ({
  useWorkspaceStore: () => ({
    environments: ENVS,
    activeEnvId: 'e-dev',
    project: { id: 'p1' },
    globalVariables: [],
    globalParams: [],
    updateEnvironment,
  }),
}))

vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn() }),
}))

vi.mock('../composables/useMiyoApi', () => ({
  useMiyoApi: () => new Proxy({}, { get: () => vi.fn() }),
}))

function $(sel: string): Element | null {
  return document.querySelector(sel)
}

async function mountDlg() {
  const wrapper = mount(EnvironmentManager, { props: { open: false }, attachTo: document.body })
  // 打开 watch 只在 open 变化时触发：挂载后再置 true
  await wrapper.setProps({ open: true })
  await nextTick()
  return wrapper
}

async function clickAddEnv() {
  const btn = $('.em-add')
  expect(btn).toBeTruthy()
  ;(btn as HTMLElement).click()
  await nextTick()
}

async function clickClose() {
  const btn = document.querySelector<HTMLButtonElement>('.m-head button.ib')
  expect(btn).toBeTruthy()
  btn!.click()
  await nextTick()
}

function renameInput(): HTMLInputElement | null {
  return document.querySelector<HTMLInputElement>('.em-row-input')
}

/** 行内重命名输入框获得焦点需要两拍 nextTick（让过 Modal 的 autofocus）。 */
async function settled() {
  await nextTick()
  await nextTick()
}

function confirmShown(): boolean {
  return $('.em-confirm') != null
}

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  updateEnvironment.mockClear()
  // 测试间重置 mock store 的环境列表（三选确认用例会追加第二个环境）
  ENVS.splice(1)
  document.body.innerHTML = ''
})

describe('EnvironmentManager 新建环境未保存确认', () => {
  it('新建后未编辑：直接关闭不弹确认，弹窗正常关闭', async () => {
    const wrapper = await mountDlg()
    await clickAddEnv()
    await settled()
    expect(confirmShown()).toBe(false)
    // 左侧行名进入行内编辑，默认名在 input value 中（textContent 不含）
    const input = renameInput()
    expect(input).toBeTruthy()
    expect(input!.value).toBe('新环境')
    expect(document.activeElement).toBe(input)

    await clickClose()
    expect(confirmShown()).toBe(false)
    expect(wrapper.emitted('update:open')?.at(-1)).toEqual([false])
  })

  it('新建后编辑名称：关闭弹出确认，弹窗保持打开', async () => {
    const wrapper = await mountDlg()
    await clickAddEnv()

    const name = document.querySelector<HTMLInputElement>('.em-name')
    expect(name).toBeTruthy()
    name!.value = '预发环境'
    name!.dispatchEvent(new Event('input'))
    await nextTick()

    await clickClose()
    expect(confirmShown()).toBe(true)
    expect(wrapper.emitted('update:open')).toBeUndefined()
  })

  it('新建后保存：关闭不弹确认', async () => {
    const wrapper = await mountDlg()
    await clickAddEnv()
    await settled()

    const save = document.querySelector<HTMLButtonElement>('.em-savebar .miyo-btn-primary')
    expect(save).toBeTruthy()
    ;(save as HTMLButtonElement).click()
    await flushPromises()
    expect(updateEnvironment).toHaveBeenCalledTimes(1)
    expect(updateEnvironment.mock.calls[0][0].name).toBe('新环境')

    await clickClose()
    expect(confirmShown()).toBe(false)
    expect(wrapper.emitted('update:open')?.at(-1)).toEqual([false])
  })
})

describe('EnvironmentManager 左侧行内重命名', () => {
  it('行内改名回车提交：左右名称同步且关闭弹确认', async () => {
    const wrapper = await mountDlg()
    await clickAddEnv()
    await settled()

    const input = renameInput()!
    input.value = '测试环境'
    input.dispatchEvent(new Event('input'))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await nextTick()

    expect(renameInput()).toBeNull()
    // 左侧行显示新名（选中行走 selected.name）
    expect($('.em-row.sel .em-row-name')?.textContent).toBe('测试环境')
    // 右侧名称同步
    expect((document.querySelector('.em-name') as HTMLInputElement).value).toBe('测试环境')

    await clickClose()
    expect(confirmShown()).toBe(true)
    expect(wrapper.emitted('update:open')).toBeUndefined()
  })

  it('行内未改名回车：不置脏，关闭不弹确认', async () => {
    const wrapper = await mountDlg()
    await clickAddEnv()
    await settled()

    const input = renameInput()!
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await nextTick()
    expect(renameInput()).toBeNull()

    await clickClose()
    expect(confirmShown()).toBe(false)
    expect(wrapper.emitted('update:open')?.at(-1)).toEqual([false])
  })

  it('Esc 取消行内改名：名称回滚、不置脏', async () => {
    const wrapper = await mountDlg()
    await clickAddEnv()
    await settled()

    const input = renameInput()!
    input.value = '改了个名'
    input.dispatchEvent(new Event('input'))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await nextTick()

    expect(renameInput()).toBeNull()
    expect($('.em-row.sel .em-row-name')?.textContent).toBe('新环境')

    await clickClose()
    expect(confirmShown()).toBe(false)
    expect(wrapper.emitted('update:open')?.at(-1)).toEqual([false])
  })

  it('双击已有行进入行内改名，回车提交并置脏', async () => {
    const wrapper = await mountDlg()
    const row = document.querySelector<HTMLElement>('.em-row')
    expect(row).toBeTruthy()
    row!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    await settled()

    const input = renameInput()!
    expect(input.value).toBe('开发环境')
    input.value = '开发环境2'
    input.dispatchEvent(new Event('input'))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await nextTick()

    expect($('.em-row.sel .em-row-name')?.textContent).toBe('开发环境2')

    await clickClose()
    expect(confirmShown()).toBe(true)
    expect(wrapper.emitted('update:open')).toBeUndefined()
  })

  it('右侧改名后左侧行实时跟随', async () => {
    await mountDlg()
    const name = document.querySelector<HTMLInputElement>('.em-name')
    name!.value = '预发环境'
    name!.dispatchEvent(new Event('input'))
    await nextTick()
    expect($('.em-row.sel .em-row-name')?.textContent).toBe('预发环境')
  })
})

describe('EnvironmentManager 未保存三选确认', () => {
  /** 追加第二个环境，供切换用例选中。 */
  function addTestEnv(): void {
    ENVS.push({
      id: 'e-test',
      project_id: 'p1',
      name: '测试环境',
      base_url: '',
      variables: [],
      created_at: '',
      updated_at: '',
    })
  }

  async function editName(value: string) {
    const name = document.querySelector<HTMLInputElement>('.em-name')!
    name.value = value
    name.dispatchEvent(new Event('input'))
    await nextTick()
  }

  async function clickRow(index: number) {
    const rows = document.querySelectorAll<HTMLElement>('.em-row')
    rows[index]!.click()
    await nextTick()
  }

  it('编辑后点其他环境：弹三选确认，「保存并继续」落库并切换', async () => {
    addTestEnv()
    await mountDlg()
    await editName('改过的环境')
    await clickRow(1)
    expect(confirmShown()).toBe(true)

    const saveBtn = document.querySelector<HTMLButtonElement>('.em-confirm .miyo-btn-primary')
    expect(saveBtn?.textContent).toContain('保存并继续')
    saveBtn!.click()
    await flushPromises()
    await nextTick()

    expect(updateEnvironment).toHaveBeenCalledTimes(1)
    expect(updateEnvironment.mock.calls[0][0].name).toBe('改过的环境')
    expect($('.em-row.sel .em-row-name')?.textContent).toBe('测试环境')
    expect(confirmShown()).toBe(false)
  })

  it('「放弃修改」不落库直接切换', async () => {
    addTestEnv()
    await mountDlg()
    await editName('改过的环境')
    await clickRow(1)
    expect(confirmShown()).toBe(true)

    const discardBtn = document.querySelector<HTMLButtonElement>('.em-confirm .miyo-btn-danger')!
    discardBtn.click()
    await nextTick()

    expect(updateEnvironment).not.toHaveBeenCalled()
    expect($('.em-row.sel .em-row-name')?.textContent).toBe('测试环境')
    expect(confirmShown()).toBe(false)
  })

  it('「继续编辑」留在当前环境且修改保留', async () => {
    addTestEnv()
    await mountDlg()
    await editName('改过的环境')
    await clickRow(1)

    const keepBtn = document.querySelector<HTMLButtonElement>('.em-confirm-actions .miyo-btn:last-child')!
    keepBtn.click()
    await nextTick()

    expect(updateEnvironment).not.toHaveBeenCalled()
    expect($('.em-row.sel .em-row-name')?.textContent).toBe('改过的环境')
    expect(confirmShown()).toBe(false)
  })
})
