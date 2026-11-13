/**
 * useShortcuts 单测：注册 / 匹配 / 注销 / 输入框隔离 / 自定义键位。
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  SHORTCUT_DEFAULTS,
  bindingLabel,
  defaultBindingOf,
  findBindingConflict,
  isShortcutCustomized,
  registerShortcut,
  resetAllShortcutBindings,
  setShortcutBinding,
  shortcutDef,
  shortcutGroups,
  shortcutLabel,
} from '@/composables/useShortcuts'

function keydown(init: KeyboardEventInit): void {
  window.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }))
}

afterEach(() => {
  resetAllShortcutBindings()
  localStorage.removeItem('miyo:shortcut-bindings')
})

describe('useShortcuts', () => {
  it('修饰键 + 主键匹配并 preventDefault', () => {
    const handler = vi.fn()
    const unregister = registerShortcut({
      id: 'test.save',
      key: 's',
      group: '测试',
      description: '保存',
      handler,
    })
    keydown({ key: 's', ctrlKey: true })
    expect(handler).toHaveBeenCalledTimes(1)
    unregister()
    keydown({ key: 's', ctrlKey: true })
    expect(handler).toHaveBeenCalledTimes(1)
  })

  it('输入框内默认不触发，inInput 显式开启才触发', () => {
    const outer = vi.fn()
    const inner = vi.fn()
    const u1 = registerShortcut({ id: 't.outer', key: 'k', group: 'G', description: 'd', handler: outer })
    const u2 = registerShortcut({
      id: 't.inner',
      key: 'j',
      group: 'G',
      description: 'd',
      inInput: true,
      handler: inner,
    })
    const input = document.createElement('input')
    document.body.appendChild(input)
    // vitest 里直接派发带 target 的事件
    input.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, key: 'k', ctrlKey: true }))
    expect(outer).not.toHaveBeenCalled()
    input.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, key: 'j', ctrlKey: true }))
    expect(inner).toHaveBeenCalledTimes(1)
    document.body.removeChild(input)
    u1()
    u2()
  })

  it('帮助数据源按组聚合 + 按键文案', () => {
    const u = registerShortcut({ id: 't.help', key: 'Enter', group: '测试组', description: '回车', handler: () => {} })
    const groups = shortcutGroups()
    const g = groups.find((x) => x.group === '测试组')
    expect(g?.items.map((i) => i.id)).toContain('t.help')
    expect(shortcutLabel({ id: 'x', key: 's', group: 'g', description: 'd', handler: () => {} })).toBe(
      '⌘/Ctrl + S',
    )
    u()
  })

  it('shortcutDef 取默认表构造注册项，未知 id 抛错', () => {
    const def = shortcutDef('editor.send', () => {})
    expect(def.key).toBe('Enter')
    expect(def.group).toBe('shortcutGroup.editor')
    expect(def.description).toBe('shortcut.send')
    expect(def.inInput).toBe(true)
    expect(() => shortcutDef('nope', () => {})).toThrow()
  })

  it('自定义覆盖即时改变匹配与展示文案', () => {
    const handler = vi.fn()
    const unregister = registerShortcut(shortcutDef('editor.save', handler))
    // 默认 Ctrl+S 生效
    keydown({ key: 's', ctrlKey: true })
    expect(handler).toHaveBeenCalledTimes(1)

    setShortcutBinding('editor.save', { mod: 'ctrl', shift: true, alt: false, key: 's' })
    expect(isShortcutCustomized('editor.save')).toBe(true)
    // 旧组合失效，新组合生效
    keydown({ key: 's', ctrlKey: true })
    expect(handler).toHaveBeenCalledTimes(1)
    keydown({ key: 'S', ctrlKey: true, shiftKey: true })
    expect(handler).toHaveBeenCalledTimes(2)
    // 展示文案跟随自定义
    expect(
      shortcutLabel({ id: 'editor.save', key: 's', group: 'g', description: 'd', handler: () => {} }),
    ).toBe('⌘/Ctrl + Shift + S')
    expect(bindingLabel(defaultBindingOf('editor.save')!)).toBe('⌘/Ctrl + Shift + S')
    unregister()
  })

  it('冲突检测：同组合返回对方定义，不同组合无冲突', () => {
    const conflict = findBindingConflict('editor.save', {
      mod: 'ctrl',
      shift: false,
      alt: false,
      key: 'Enter',
    })
    expect(conflict?.id).toBe('editor.send')
    // 自身不与自身冲突
    expect(
      findBindingConflict('editor.send', { mod: 'ctrl', shift: false, alt: false, key: 'Enter' }),
    ).toBeNull()
    // 修饰键不同即无冲突
    expect(
      findBindingConflict('editor.save', { mod: 'ctrl', shift: true, alt: false, key: 's' }),
    ).toBeNull()
  })

  it('覆盖持久化到 localStorage，未知 id 被忽略', () => {
    setShortcutBinding('editor.send', { mod: 'ctrl', shift: false, alt: true, key: 'Enter' })
    const raw = localStorage.getItem('miyo:shortcut-bindings')
    expect(raw).toContain('editor.send')
    setShortcutBinding('unknown.id', { mod: 'ctrl', shift: false, alt: false, key: 'x' })
    expect(defaultBindingOf('unknown.id')).toBeNull()
    expect(isShortcutCustomized('unknown.id')).toBe(false)
  })

  it('默认表 id 唯一', () => {
    const ids = SHORTCUT_DEFAULTS.map((d) => d.id)
    expect(new Set(ids).size).toBe(ids.length)
  })
})
