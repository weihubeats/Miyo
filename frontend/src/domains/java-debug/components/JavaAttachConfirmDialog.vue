<script setup lang="ts">
/**
 * JavaAttachConfirmDialog：attach 前的强制二次确认（安全控制）。
 * 明确告知：Agent 将动态注入目标 JVM（运行时受影响）、仅限本机回环通信、
 * 一次性 token、可随时断开 / 卸载；已加载的类无法卸载（诚实边界）。
 *
 * 壳层走共享 Modal.vue（此前自绘 fixed 遮罩 + 弹层，视觉/焦点/Esc 语义
 * 与全站弹窗不一致）；内容区仅保留本弹窗专属排版。
 */
import { computed } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import type { JavaProcessInfo } from '@/types/miyoApi'
import Modal from '@/components/ui/Modal.vue'

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
  <Modal
    :open="open"
    :title="t('java2.attachConfirmTitle')"
    width="440px"
    @close="emit('close')"
  >
    <div class="jac" data-testid="java-attach-confirm">
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
    </div>
    <template #footer>
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
    </template>
  </Modal>
</template>

<style scoped>
.jac {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.jac-target {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.jac-name {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
}
.jac-chip {
  font-size: var(--fs-xxs);
  padding: 2px 8px;
  border-radius: var(--radius-pill);
  background: var(--bg-hover);
  color: var(--text-2);
}
.jac-points {
  margin: 0;
  padding-left: 18px;
  font-size: var(--fs-xs);
  color: var(--text-2);
  line-height: 1.7;
}
.jac-warn {
  font-size: var(--fs-xxs);
  padding: 8px 10px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-2);
  line-height: 1.5;
}
</style>
