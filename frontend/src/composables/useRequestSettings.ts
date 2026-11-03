/**
 * useRequestSettings：请求设置 Popover 状态机（自 EndpointEditor 抽出）。
 *
 * 职责（原 EndpointEditor onTimeoutInput … onSettingsDocKeydown 整段，行为不变）：
 * - 超时输入解析（空串 = 跟随全局 null；非法输入不写回，避免打字中途被清）；
 * - 超时占位文案（未设置时展示全局默认值，让「跟随全局」具体可见）；
 * - Popover 定位（请求行触发钮上/下翻边界计算）与关闭交互
 *   （滚动 / 缩放 / 外部点击 / Esc；触发钮自行接管开关，面板内操作不关）；
 * - 非默认值（设了超时或关闭重定向）时触发钮点亮角标（hasCustomSettings）。
 *
 * 面板 Teleport 到 body + fixed 定位（config-box 是 overflow 滚动容器，
 * 内部绝对定位会被裁剪）。窗口级监听由 dispose() 统一摘除。
 */
import { computed, ref, type ComputedRef, type Ref } from 'vue'
import type { Endpoint, RequestSpec } from '../types/miyoApi'

/** 定位用估算尺寸：与 .rs-pop 的 width 一致；高度略放宽只影响上翻判定，宁大勿小。 */
const SETTINGS_POPOVER = { width: 220, height: 110 }

export interface UseRequestSettingsOptions {
  /** 当前草稿接口（写 request.timeout_ms / 读 follow_redirects）。 */
  draft: ComputedRef<Endpoint | null>
  /** 全局超时毫秒数（null = 未加载 / 未配置）。 */
  globalTimeoutMs: Ref<number | null | undefined>
  /** 文案函数（组件 setup 内的 locale.t）。 */
  t: (key: string, params?: Record<string, string | number>) => string
  /** 触发钮锚点（DOM 归宿主持有，string ref 模板关联需要独立声明）。 */
  settingsTriggerEl: Ref<HTMLElement | null>
}

export function useRequestSettings(options: UseRequestSettingsOptions) {
  const { draft, globalTimeoutMs, t, settingsTriggerEl } = options

  /** 单请求超时输入：空串 = null（跟随全局超时），非法输入不写回（避免打字中途被清空）。 */
  function onTimeoutInput(value: string | number): void {
    const request = draft.value?.request
    if (!request) return
    if (value === '' || value === null || value === undefined) {
      request.timeout_ms = null
      return
    }
    const n = typeof value === 'number' ? value : Number(value)
    if (Number.isFinite(n) && n > 0) request.timeout_ms = Math.round(n)
  }

  /** 超时占位：未设置（留空）时展示全局默认值，让「跟随全局」具体可见；未加载完成回退静态文案。 */
  const timeoutPlaceholder = computed(() => {
    const ms = globalTimeoutMs.value
    return ms == null ? t('editor.timeoutPh') : t('editor.timeoutPhGlobal', { sec: Math.round(ms / 1000) })
  })

  /** 请求设置 Popover：超时/重定向是低频设置，收进页签行右侧触发钮——
   *  请求与响应之间的接缝处不再常驻一行（用户实测：夹在 Body 与响应之间显乱）。 */
  const settingsOpen = ref(false)
  const settingsPos = ref({ left: 0, top: 0 })

  const hasCustomSettings = computed(() => {
    const request: RequestSpec | undefined = draft.value?.request
    return !!request && (request.timeout_ms != null || request.follow_redirects === false)
  })

  function toggleSettings(): void {
    if (settingsOpen.value) {
      settingsOpen.value = false
      return
    }
    const rect = settingsTriggerEl.value?.getBoundingClientRect()
    if (!rect) return
    const up =
      window.innerHeight - rect.bottom < SETTINGS_POPOVER.height + 8 && rect.top > SETTINGS_POPOVER.height
    settingsPos.value = {
      left: Math.max(
        8,
        Math.min(rect.right - SETTINGS_POPOVER.width, window.innerWidth - SETTINGS_POPOVER.width - 8),
      ),
      top: up ? rect.top - SETTINGS_POPOVER.height - 6 : rect.bottom + 6,
    }
    settingsOpen.value = true
  }

  function closeSettings(): void {
    settingsOpen.value = false
  }

  function onSettingsDocMousedown(event: MouseEvent): void {
    if (!settingsOpen.value) return
    const target = event.target as Node
    // 触发钮自行接管开关；面板本体（Teleport 出去）内的操作不关
    if (settingsTriggerEl.value?.contains(target)) return
    if (target instanceof Element && target.closest('[data-settings-pop]')) return
    closeSettings()
  }

  function onSettingsDocKeydown(event: KeyboardEvent): void {
    if (settingsOpen.value && (event.key === 'Escape' || event.key === 'Tab')) closeSettings()
  }

  window.addEventListener('mousedown', onSettingsDocMousedown)
  window.addEventListener('keydown', onSettingsDocKeydown)
  window.addEventListener('scroll', closeSettings, true)
  window.addEventListener('resize', closeSettings)

  /** 卸载清理：摘除窗口级监听（宿主 onUnmounted 调用）。 */
  function dispose(): void {
    window.removeEventListener('mousedown', onSettingsDocMousedown)
    window.removeEventListener('keydown', onSettingsDocKeydown)
    window.removeEventListener('scroll', closeSettings, true)
    window.removeEventListener('resize', closeSettings)
  }

  return {
    onTimeoutInput,
    timeoutPlaceholder,
    settingsTriggerEl,
    settingsOpen,
    settingsPos,
    hasCustomSettings,
    toggleSettings,
    closeSettings,
    dispose,
  }
}
