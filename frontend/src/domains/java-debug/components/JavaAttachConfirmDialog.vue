<script setup lang="ts">
/**
 * JavaAttachConfirmDialog：attach 前的强制二次确认（安全控制）。
 * 明确告知：Agent 将动态注入目标 JVM（运行时受影响）、仅限本机回环通信、
 * 一次性 token、可随时断开 / 卸载；已加载的类无法卸载（诚实边界）。
 */
import { computed } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import type { JavaProcessInfo } from '@/types/miyoApi'

const props = defineProps<{
  open: boolean
  target: JavaProcessInfo | null
  busy: boolean
}>()

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'confirm'): void
}>()

const t = useLocaleStore().t

const displayName = computed(() =>
  props.target ? (props.target.display_name || `PID ${props.target.pid}`) : '',
)
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="jac-mask" @click.self="emit('close')">
      <div class="jac" role="dialog" aria-modal="true" data-testid="java-attach-confirm">
        <h2 class="jac-title">{{ t('java2.attachConfirmTitle') }}</h2>
        <div class="jac-target" v-if="target">
          <span class="jac-name">{{ displayName }}</span>
          <span class="jac-chip">PID {{ target.pid }}</span>
          <span class="jac-chip">{{ target.java_version || target.kind }}</span>
        </div>
        <ul class="jac-points">
          <li>{{ t('java2.attachPointInject') }}</li>
          <li>{{ t('java2.attachPointLoopback') }}</li>
          <li>{{ t('java2.attachPointToken') }}</li>
          <li>{{ t('java2.attachPointUnload') }}</li>
        </ul>
        <div class="jac-warn">{{ t('java2.attachWarn') }}</div>
        <div class="jac-actions">
          <button type="button" class="miyo-btn" data-testid="java-attach-cancel" @click="emit('close')">
            {{ t('common.cancel') }}
          </button>
          <button
            type="button"
            class="miyo-btn miyo-btn-primary"
            :disabled="busy"
            data-testid="java-attach-confirm-btn"
            @click="emit('confirm')"
          >
            {{ busy ? t('java2.attaching') : t('java2.attachConfirmOk') }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.jac-mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}
.jac {
  width: 440px;
  max-width: calc(100vw - 32px);
  background: var(--bg-elevated, var(--bg));
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.jac-title {
  margin: 0;
  font-size: 14.5px;
  font-weight: 600;
  color: var(--text);
}
.jac-target {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.jac-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}
.jac-chip {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-secondary);
}
.jac-points {
  margin: 0;
  padding-left: 18px;
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.7;
}
.jac-warn {
  font-size: 11.5px;
  padding: 8px 10px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-secondary);
  line-height: 1.5;
}
.jac-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
