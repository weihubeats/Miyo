/**
 * useShortcuts：全局快捷键集中注册表。
 *
 * 背景：快捷键原来散落在各组件的 window keydown 监听里（EndpointEditor、
 * ResponsePanel…），无统一展示、无冲突检查。集中注册后：
 * - 同一按键只响应最后注册（栈顶优先），卸载自动注销，无泄漏；
 * - 快捷键帮助面板直接由注册表生成，新增快捷键自动出现在帮助里；
 * - 输入框内（INPUT/TEXTAREA/contentEditable）默认不触发全局动作，
 *   需要的动作显式声明 `inInput: true`。
 * - 键位可在设置页自定义：SHORTCUT_DEFAULTS 为唯一默认源，组件注册时
 *   引用它；用户覆盖存 localStorage，匹配与展示统一走生效键位。
 */
import { onBeforeUnmount, ref } from 'vue'

export interface ShortcutDef {
  /** 唯一标识（重复注册时后者覆盖前者）。 */
  id: string
  /** 修饰键（ctrl/meta 二选一或都不要；macOS 上 ctrl 视为 ⌘）。 */
  mod?: 'ctrl' | 'meta' | 'none'
  shift?: boolean
  alt?: boolean
  /** 主键（event.key，不区分大小写比较）。 */
  key: string
  /** 展示分组（帮助面板用）。 */
  group: string
  /** 展示文案。 */
  description: string
  /** 输入框内是否触发（默认 false）。 */
  inInput?: boolean
  handler: (event: KeyboardEvent) => void
}

const registry = new Map<string, ShortcutDef>()
let listening = false

// ---------- 自定义键位（设置页） ----------
/** 可绑定的按键组合（不含 handler，供设置页展示/录制）。 */
export interface ShortcutBinding {
  mod: 'ctrl' | 'meta' | 'none'
  shift: boolean
  alt: boolean
  key: string
}

/** 可自定义快捷键的默认表（唯一默认源；组件注册时引用，设置页直接展示）。
 * group / description 存 i18n 键，展示时经 locale.t 解析。 */
export interface ShortcutDefault {
  id: string
  group: string
  description: string
  inInput?: boolean
  binding: ShortcutBinding
}

export const SHORTCUT_DEFAULTS: ShortcutDefault[] = [
  {
    id: 'workspace.shortcuts-help',
    group: 'shortcutGroup.common',
    description: 'shortcut.help',
    binding: { mod: 'ctrl', shift: false, alt: false, key: '/' },
  },
  {
    id: 'workspace.quick-search',
    group: 'shortcutGroup.common',
    description: 'shortcut.quickSearch',
    // 命令面板的意义就在「任何地方可唤起」：地址栏 / 表单聚焦时同样生效。
    inInput: true,
    binding: { mod: 'ctrl', shift: false, alt: false, key: 'k' },
  },
  {
    id: 'editor.save',
    group: 'shortcutGroup.editor',
    description: 'shortcut.save',
    inInput: true,
    binding: { mod: 'ctrl', shift: false, alt: false, key: 's' },
  },
  {
    id: 'editor.send',
    group: 'shortcutGroup.editor',
    description: 'shortcut.send',
    inInput: true,
    binding: { mod: 'ctrl', shift: false, alt: false, key: 'Enter' },
  },
  {
    id: 'editor.new-request-t',
    group: 'shortcutGroup.editor',
    description: 'shortcut.newRequest',
    inInput: true,
    binding: { mod: 'ctrl', shift: false, alt: false, key: 't' },
  },
  {
    id: 'editor.new-request-n',
    group: 'shortcutGroup.editor',
    description: 'shortcut.newRequest',
    inInput: true,
    binding: { mod: 'ctrl', shift: false, alt: false, key: 'n' },
  },
]

const BINDINGS_KEY = 'miyo:shortcut-bindings'

function readOverrides(): Record<string, ShortcutBinding> {
  try {
    const raw = localStorage.getItem(BINDINGS_KEY)
    if (!raw) return {}
    const parsed = JSON.parse(raw) as Record<string, unknown>
    const out: Record<string, ShortcutBinding> = {}
    for (const d of SHORTCUT_DEFAULTS) {
      const v = parsed[d.id] as Partial<ShortcutBinding> | undefined
      if (!v || typeof v.key !== 'string' || !v.key) continue
      out[d.id] = {
        mod: v.mod === 'meta' || v.mod === 'none' ? v.mod : 'ctrl',
        shift: v.shift === true,
        alt: v.alt === true,
        key: v.key,
      }
    }
    return out
  } catch {
    return {}
  }
}

let bindingOverrides: Record<string, ShortcutBinding> = readOverrides()

function persistOverrides(): void {
  try {
    localStorage.setItem(BINDINGS_KEY, JSON.stringify(bindingOverrides))
  } catch {
    // 存储不可用时仅内存生效
  }
}

/** 覆盖变更计数（设置页据此刷新展示）。 */
export const shortcutBindingsTick = ref(0)

/** 注册表项的生效键位（用户覆盖优先，否则用注册时的键位）。 */
export function effectiveBindingOf(def: ShortcutDef): ShortcutBinding {
  const o = bindingOverrides[def.id]
  if (o) return o
  return {
    mod: def.mod ?? 'ctrl',
    shift: !!def.shift,
    alt: !!def.alt,
    key: def.key,
  }
}

/** 默认表中某 id 的生效键位（设置页展示用；未注册时也可用）。 */
export function defaultBindingOf(id: string): ShortcutBinding | null {
  const o = bindingOverrides[id]
  if (o) return o
  return SHORTCUT_DEFAULTS.find((d) => d.id === id)?.binding ?? null
}

/** 设置用户键位（持久化并通知刷新）。 */
export function setShortcutBinding(id: string, binding: ShortcutBinding): void {
  if (!SHORTCUT_DEFAULTS.some((d) => d.id === id)) return
  bindingOverrides[id] = { ...binding }
  persistOverrides()
  shortcutBindingsTick.value += 1
}

/** 恢复单个默认键位。 */
export function resetShortcutBinding(id: string): void {
  delete bindingOverrides[id]
  persistOverrides()
  shortcutBindingsTick.value += 1
}

/** 恢复全部默认键位。 */
export function resetAllShortcutBindings(): void {
  bindingOverrides = {}
  persistOverrides()
  shortcutBindingsTick.value += 1
}

/** 是否被用户改过。 */
export function isShortcutCustomized(id: string): boolean {
  return id in bindingOverrides
}

/** 由默认表构造注册项（组件注册时使用，默认键位单源）。 */
export function shortcutDef(id: string, handler: (event: KeyboardEvent) => void): ShortcutDef {
  const d = SHORTCUT_DEFAULTS.find((x) => x.id === id)
  if (!d) throw new Error(`unknown shortcut: ${id}`)
  return {
    id: d.id,
    key: d.binding.key,
    mod: d.binding.mod,
    shift: d.binding.shift,
    alt: d.binding.alt,
    group: d.group,
    description: d.description,
    inInput: d.inInput,
    handler,
  }
}

/** 组合归一化（macOS 上 Ctrl/⌘ 互通，冲突判断视为同一修饰键）。 */
export function bindingCombo(b: ShortcutBinding): string {
  const mod = b.mod === 'none' ? 'none' : 'mod'
  return [mod, b.shift ? 'shift' : '', b.alt ? 'alt' : '', b.key.toLowerCase()].join('+')
}

/** 查找与目标组合冲突的其他可自定义项（返回对方默认定义，无则 null）。 */
export function findBindingConflict(id: string, binding: ShortcutBinding): ShortcutDefault | null {
  const combo = bindingCombo(binding)
  for (const d of SHORTCUT_DEFAULTS) {
    if (d.id === id) continue
    const eff = bindingOverrides[d.id] ?? d.binding
    if (bindingCombo(eff) === combo) return d
  }
  return null
}

function isEditable(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null
  if (!el || typeof (el as HTMLElement).tagName !== 'string') return false
  const tag = (el as HTMLElement).tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || (el as HTMLElement).isContentEditable
}

function modOk(binding: ShortcutBinding, e: KeyboardEvent): boolean {
  if (binding.mod === 'none') return !e.ctrlKey && !e.metaKey
  // macOS 上 Ctrl 与 ⌘ 互通：任一按下即视为修饰符满足。
  return e.ctrlKey || e.metaKey
}

function onKeydown(e: KeyboardEvent): void {
  for (const def of [...registry.values()].reverse()) {
    const b = effectiveBindingOf(def)
    if (!modOk(b, e)) continue
    if (!!b.shift !== e.shiftKey) continue
    if (!!b.alt !== e.altKey) continue
    if (b.key.toLowerCase() !== e.key.toLowerCase()) continue
    if (!def.inInput && isEditable(e.target)) continue
    e.preventDefault()
    def.handler(e)
    return
  }
}

function ensureListening(): void {
  if (listening) return
  listening = true
  window.addEventListener('keydown', onKeydown)
}

/** 注册全局快捷键（组件卸载时自动注销；同 id 后注册覆盖）。 */
export function registerShortcut(def: ShortcutDef): () => void {
  ensureListening()
  registry.set(def.id, def)
  return () => {
    if (registry.get(def.id) === def) registry.delete(def.id)
  }
}

/** 快捷键帮助面板的数据源（按 group 分组，保持注册顺序）。 */
export function shortcutGroups(): Array<{ group: string; items: ShortcutDef[] }> {
  const groups = new Map<string, ShortcutDef[]>()
  for (const def of registry.values()) {
    const list = groups.get(def.group)
    if (list) list.push(def)
    else groups.set(def.group, [def])
  }
  return [...groups.entries()].map(([group, items]) => ({ group, items }))
}

/** 展示用按键文案（⌘/Ctrl + Shift + Key），用户自定义键位自动生效。 */
export function shortcutLabel(def: ShortcutDef): string {
  return bindingLabel(effectiveBindingOf(def))
}

/** 键位组合展示文案（设置页录制态共用）。 */
export function bindingLabel(b: ShortcutBinding): string {
  const parts: string[] = []
  if (b.mod !== 'none') parts.push('⌘/Ctrl')
  if (b.shift) parts.push('Shift')
  if (b.alt) parts.push('Alt')
  parts.push(prettyKey(b.key))
  return parts.join(' + ')
}

/** 主键展示美化（空格 / 单字母大写）。 */
export function prettyKey(key: string): string {
  if (key === ' ') return 'Space'
  return key.length === 1 ? key.toUpperCase() : key
}

/** 在 setup 中注册（卸载自动注销）。 */
export function useShortcuts(defs: ShortcutDef[]): void {
  const unregisters = defs.map(registerShortcut)
  onBeforeUnmount(() => {
    for (const u of unregisters) u()
  })
}
