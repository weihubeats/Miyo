/**
 * skills store 单测：扫描填充、启停就地替换、项目作用域缺目录时跳过扫描。
 * useFoxApi 以模块级 mock 替换（内存假后端）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import type { SkillIndex, SkillSummary } from '../types/foxApi'

const api = vi.hoisted(() => ({
  skillsScan: vi.fn(),
  skillSetEnabled: vi.fn(),
  skillDirPath: vi.fn(),
  skillRead: vi.fn(),
  skillCreate: vi.fn(),
  skillSave: vi.fn(),
  skillDelete: vi.fn(),
}))

vi.mock('../composables/useFoxApi', () => ({
  useFoxApi: () => api,
}))

import { useSkillsStore } from './skills'

function summary(name: string, state: 'linked' | 'missing'): SkillSummary {
  return {
    name,
    description: `${name} desc`,
    sourceRoot: '/home/u/.agents/skills',
    dir: `/home/u/.agents/skills/${name}`,
    updatedAt: 1,
    extraEntries: [],
    states: [
      {
        toolId: 'claude-code',
        toolName: 'Claude Code',
        root: '/home/u/.claude/skills',
        state,
        detail: null,
      },
    ],
    warnings: [],
  }
}

function indexOf(skills: SkillSummary[]): SkillIndex {
  return {
    sourceRoot: '/home/u/.agents/skills',
    skills,
    foreign: [],
    tools: [],
    warnings: [],
  }
}

beforeEach(() => {
  setActivePinia(createPinia())
  api.skillsScan.mockReset()
  api.skillSetEnabled.mockReset()
  api.skillDirPath.mockReset()
  api.skillRead.mockReset()
  api.skillCreate.mockReset()
  api.skillSave.mockReset()
  api.skillDelete.mockReset()
})

describe('skills store', () => {
  it('refresh 填充索引并复位 loading', async () => {
    api.skillsScan.mockResolvedValue(indexOf([summary('demo', 'missing')]))
    const store = useSkillsStore()

    await store.refresh()

    expect(api.skillsScan).toHaveBeenCalledWith('global', null)
    expect(store.skills.map((skill) => skill.name)).toEqual(['demo'])
    expect(store.loading).toBe(false)
    expect(store.error).toBe('')
  })

  it('启停成功后只替换对应行', async () => {
    api.skillsScan.mockResolvedValue(
      indexOf([summary('a', 'missing'), summary('b', 'missing')]),
    )
    const store = useSkillsStore()
    await store.refresh()

    api.skillSetEnabled.mockResolvedValue(summary('a', 'linked'))
    await store.setEnabled('a', 'claude-code', true)

    expect(api.skillSetEnabled).toHaveBeenCalledWith('a', 'claude-code', true, 'global', null)
    expect(store.skills[0].states[0].state).toBe('linked')
    expect(store.skills[1].states[0].state).toBe('missing')
  })

  it('项目作用域未选目录时不扫描', async () => {
    const store = useSkillsStore()
    store.setScope('project')
    await store.refresh()

    expect(api.skillsScan).not.toHaveBeenCalled()
    expect(store.index).toBeNull()
  })

  it('扫描失败设置 error 并向上抛', async () => {
    api.skillsScan.mockRejectedValue(new Error('boom'))
    const store = useSkillsStore()

    await expect(store.refresh()).rejects.toThrow('boom')
    expect(store.error).toBe('boom')
    expect(store.loading).toBe(false)
  })

  it('新建后插入并保持名称有序', async () => {
    api.skillsScan.mockResolvedValue(indexOf([summary('zeta', 'missing')]))
    const store = useSkillsStore()
    await store.refresh()

    api.skillCreate.mockResolvedValue(summary('alpha', 'missing'))
    await store.create({ name: 'alpha', description: 'a', body: 'b' })

    expect(store.skills.map((skill) => skill.name)).toEqual(['alpha', 'zeta'])
  })

  it('保存后替换对应行', async () => {
    api.skillsScan.mockResolvedValue(indexOf([summary('demo', 'missing')]))
    const store = useSkillsStore()
    await store.refresh()

    const updated = summary('demo', 'linked')
    updated.description = 'new desc'
    api.skillSave.mockResolvedValue(updated)
    await store.save('demo', {
      name: 'demo',
      description: 'new desc',
      body: 'b',
      rawExpected: 'raw',
    })

    expect(api.skillSave).toHaveBeenCalledWith(
      'demo',
      { name: 'demo', description: 'new desc', body: 'b', rawExpected: 'raw' },
      'global',
      null,
    )
    expect(store.skills[0].description).toBe('new desc')
  })

  it('删除后从列表移除', async () => {
    api.skillsScan.mockResolvedValue(
      indexOf([summary('a', 'missing'), summary('b', 'missing')]),
    )
    const store = useSkillsStore()
    await store.refresh()

    api.skillDelete.mockResolvedValue(undefined)
    await store.remove('a')

    expect(api.skillDelete).toHaveBeenCalledWith('a', 'global', null)
    expect(store.skills.map((skill) => skill.name)).toEqual(['b'])
  })

  it('read 透传当前作用域', async () => {
    const store = useSkillsStore()
    store.setScope('project', '/tmp/demo')
    api.skillRead.mockResolvedValue({ summary: summary('demo', 'missing') })

    await store.read('demo')

    expect(api.skillRead).toHaveBeenCalledWith('demo', 'project', '/tmp/demo')
  })
})
