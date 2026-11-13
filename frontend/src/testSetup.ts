import { preloadBootLocale } from '@/stores/locale'

/**
 * vitest setup：修复 Node ≥24 实验性全局 localStorage 残缺问题。
 *
 * Node 24/25 默认暴露原生 `localStorage` 全局（Web Storage 实验特性），未传
 * `--localstorage-file` 时后端不可用，得到的是**没有任何 Storage 方法**的空对象；
 * vitest 的 jsdom 环境合并全局时它会遮蔽 jsdom 的实现，导致所有直接调用
 * `localStorage.*` 的测试抛 `xxx is not a function`（CI 固定 Node 22 无此全局，故 CI 绿）。
 *
 * 这里检测到方法缺失时，用内存 Map 实现的 Storage 垫片替换之；功能完备时
 * （Node 22 / jsdom 正常接管）原样放行。
 */

interface StorageShim {
  readonly length: number
  key(index: number): string | null
  getItem(key: string): string | null
  setItem(key: string, value: string): void
  removeItem(key: string): void
  clear(): void
}

function isFunctionalStorage(value: unknown): boolean {
  const s = value as Partial<StorageShim> | undefined
  return (
    !!s &&
    typeof s.getItem === 'function' &&
    typeof s.setItem === 'function' &&
    typeof s.removeItem === 'function' &&
    typeof s.clear === 'function' &&
    typeof s.key === 'function'
  )
}

function memoryStorage(): StorageShim {
  const map = new Map<string, string>()
  return {
    get length() {
      return map.size
    },
    key(index: number) {
      return [...map.keys()][index] ?? null
    },
    getItem(key) {
      const k = String(key)
      return map.has(k) ? (map.get(k) as string) : null
    },
    setItem(key, value) {
      map.set(String(key), String(value))
    },
    removeItem(key) {
      map.delete(String(key))
    },
    clear() {
      map.clear()
    },
  }
}

const current = (globalThis as { localStorage?: unknown }).localStorage
if (!isFunctionalStorage(current)) {
  Object.defineProperty(globalThis, 'localStorage', {
    value: memoryStorage(),
    configurable: true,
    writable: true,
  })
}

// 启动语言字典：应用侧由 main.ts 的 bootstrap 在挂载前补齐，测试没有这一步。
// jsdom 环境语言是 en-US，不预热的话 translate('en', …) 会回落到中文文案。
await preloadBootLocale()
