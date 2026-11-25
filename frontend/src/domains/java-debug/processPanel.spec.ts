/**
 * JavaProcessPanel 单测。
 *
 * 背景（真实反馈，2026-11）：点了「断开 agent」，左侧进程列表仍显示「已连接」。
 * 根因是面板只读扫描结果里的 `p.attached`——那是后端进程扫描快照，
 * 断开后要等下一轮轮询才变（自动刷新关闭时永远不变），而 `attachedPids`
 * 直接来自会话表、`detach()` 一返回就更新。这里锁定「会话列表优先」。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import JavaProcessPanel from './components/JavaProcessPanel.vue'
import { useLocaleStore } from '@/stores/locale'
import type { JavaProcessInfo, JavaProcessListSummary } from '@/types/miyoApi'

function proc(overrides: Partial<JavaProcessInfo> = {}): JavaProcessInfo {
  return {
    pid: 70513,
    main_class: 'com.redotpay.PayApplication',
    display_name: 'com.redotpay.PayApplication',
    command_line: 'java com.redotpay.PayApplication',
    java_executable: '/jdk/bin/java',
    java_home: '/jdk',
    java_version: '17.0.16',
    jvm_vendor: 'Azul',
    kind: 'jdk',
    started_at: '',
    elapsed: '19:16',
    uid: '501',
    attachable: true,
    attach_block_reason: null,
    attached: false,
    source: 'os_scan',
    ...overrides,
  }
}

const summary: JavaProcessListSummary = {
  processes: [],
  toolchain_count: 1,
  launcher_ready: true,
  agent_ready: true,
  scan_ms: 3,
  hints: [],
}

function mountPanel(overrides: Record<string, unknown> = {}) {
  return mount(JavaProcessPanel, {
    props: {
      summary,
      processes: [proc()],
      loading: false,
      search: '',
      activePid: null,
      attachedPids: [],
      autoRefresh: false,
      componentsDownloading: false,
      ...overrides,
    },
  })
}

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  vi.clearAllMocks()
})

describe('JavaProcessPanel 已连接状态', () => {
  it('会话表里有该 pid → 立即显示「已连接」（不依赖扫描快照）', () => {
    const wrapper = mountPanel({
      processes: [proc({ attached: false })],
      attachedPids: [70513],
    })
    expect(wrapper.text()).toContain('已连接')
    // 已连接时不显示「连接」按钮
    expect(wrapper.find('[data-testid="java-attach-btn"]').exists()).toBe(false)
  })

  it('扫描快照仍说 attached=true 但会话表已空 → 以会话表为准，显示可连接', () => {
    // 断开瞬间的典型状态：detach 已返回（会话移除），扫描结果还没刷新
    const wrapper = mountPanel({
      processes: [proc({ attached: true })],
      attachedPids: [],
    })
    expect(wrapper.text()).not.toContain('已连接')
    expect(wrapper.find('[data-testid="java-attach-btn"]').exists()).toBe(true)
  })

  it('扫描快照 attached=true 且会话表有 → 仍为已连接（兜底不倒退）', () => {
    const wrapper = mountPanel({
      processes: [proc({ attached: true })],
      attachedPids: [70513],
    })
    expect(wrapper.text()).toContain('已连接')
  })

  it('从未连接 → 显示可连接 + 连接按钮', () => {
    const wrapper = mountPanel({ attachedPids: [] })
    expect(wrapper.find('[data-testid="java-attach-btn"]').exists()).toBe(true)
  })
})