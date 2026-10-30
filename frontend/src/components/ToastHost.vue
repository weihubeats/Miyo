<script setup lang="ts">
/**
 * ToastHost：渲染全局 Toast（配合 composables/useToast.ts）。
 *
 * 在 App 根组件挂载一次即可：
 * ```html
 * <ToastHost />
 * ```
 * 样式为 miyo- 设计系统的 Vue 侧镜像（scoped + 变量对齐 styles.rs：
 * --miyo-bg-panel / --miyo-border / --miyo-text / 语义色）。
 *
 * 播报策略：容器整体 aria-live="polite"（成功 / 信息 / 警告排队播报），
 * error 条目单独 role="alert" + aria-live="assertive"，插入即打断播报。
 */
import { TOAST_TYPE_META, useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import type { ToastItem, ToastType } from '../composables/useToast'

const { toasts, dismiss } = useToast()
const locale = useLocaleStore()
const t = locale.t

const ICONS: Record<ToastType, string> = {
  success:
    '<svg viewBox="0 0 16 16" width="16" height="16"><path d="M3 8.5l3.2 3.2L13 5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  info: '<svg viewBox="0 0 16 16" width="16" height="16"><circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M8 7.2v4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/><circle cx="8" cy="4.8" r="1" fill="currentColor"/></svg>',
  warning:
    '<svg viewBox="0 0 16 16" width="16" height="16"><path d="M8 2L1.5 13.5h13L8 2z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/><path d="M8 6.4v3.4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/><circle cx="8" cy="11.5" r="0.9" fill="currentColor"/></svg>',
  error:
    '<svg viewBox="0 0 16 16" width="16" height="16"><circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M5.5 5.5l5 5M10.5 5.5l-5 5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>',
}

function meta(type: ToastType): { color: string } {
  return TOAST_TYPE_META[type]
}

function onClose(item: ToastItem): void {
  dismiss(item.id)
}

/** 动作按钮：执行回调后关闭该条 toast。 */
function onAction(item: ToastItem): void {
  item.action?.run()
  dismiss(item.id)
}
</script>

<template>
  <div class="miyo-toast-wrap" aria-live="polite">
    <TransitionGroup name="miyo-toast">
      <div
        v-for="item in toasts"
        :key="item.id"
        class="miyo-toast"
        :class="`miyo-toast-${item.type}`"
        :role="item.type === 'error' ? 'alert' : undefined"
        :aria-live="item.type === 'error' ? 'assertive' : undefined"
        aria-atomic="true"
      >
        <span
          class="miyo-toast-icon"
          :style="{ color: meta(item.type).color }"
          v-html="ICONS[item.type]"
        ></span>
        <div class="miyo-toast-body">
          <div class="miyo-toast-title">{{ item.title }}</div>
          <div v-if="item.message" class="miyo-toast-message">{{ item.message }}</div>
        </div>
        <button v-if="item.action" class="miyo-toast-action" type="button" @click="onAction(item)">
          {{ item.action.label }}
        </button>
        <button
          class="miyo-toast-close"
          type="button"
          :aria-label="t('toast.dismiss', { v: item.title })"
          @click="onClose(item)"
        >
          <svg viewBox="0 0 12 12" width="12" height="12">
            <path
              d="M2.5 2.5l7 7M9.5 2.5l-7 7"
              stroke="currentColor"
              stroke-width="1.5"
              stroke-linecap="round"
            />
          </svg>
        </button>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
.miyo-toast-wrap {
  position: fixed;
  top: 12px;
  left: 50%;
  transform: translateX(-50%);
  z-index: 1000;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  width: 420px;
  max-width: calc(100vw - 32px);
  pointer-events: none;
}

.miyo-toast {
  pointer-events: auto;
  display: flex;
  align-items: flex-start;
  gap: 10px;
  width: 100%;
  padding: 10px 12px 10px 14px;
  border-radius: var(--radius);
  background: var(--bg-elevated);
  border: 1px solid var(--border-strong);
  border-left-width: 3px;
  box-shadow: var(--shadow-lg);
}

.miyo-toast-success {
  border-left-color: var(--miyo-success);
}
.miyo-toast-info {
  border-left-color: var(--miyo-info);
}
.miyo-toast-warning {
  border-left-color: var(--miyo-warning);
}
.miyo-toast-error {
  border-left-color: var(--miyo-danger);
}

.miyo-toast-icon {
  flex: none;
  margin-top: 2px;
  display: inline-flex;
}

.miyo-toast-body {
  flex: 1;
  min-width: 0;
}

.miyo-toast-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--miyo-text);
  line-height: 1.4;
}

.miyo-toast-message {
  margin-top: 2px;
  font-size: 12px;
  color: var(--miyo-text-secondary);
  line-height: 1.45;
  word-break: break-all;
}

.miyo-toast-close {
  flex: none;
  border: none;
  background: none;
  padding: 2px;
  margin-top: 1px;
  cursor: pointer;
  color: var(--miyo-text-muted);
  border-radius: var(--radius-sm);
  line-height: 1.2;
}

.miyo-toast-close:hover {
  color: var(--miyo-text);
  background: var(--miyo-hover);
}

.miyo-toast-action {
  flex: none;
  align-self: center;
  padding: 4px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 6px;
  background: var(--bg-hover);
  color: var(--text-1);
  font-family: inherit;
  font-size: var(--fs-xs);
  white-space: nowrap;
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}
.miyo-toast-action:hover {
  background: var(--accent-tint);
  border-color: color-mix(in srgb, var(--accent) 40%, transparent);
}

.miyo-toast-enter-active,
.miyo-toast-leave-active {
  transition:
    opacity 0.2s ease,
    transform 0.2s var(--ease);
}

.miyo-toast-enter-from {
  opacity: 0;
  transform: translateY(-12px) scale(0.98);
}

.miyo-toast-leave-to {
  opacity: 0;
  transform: translateY(-8px) scale(0.98);
}
</style>