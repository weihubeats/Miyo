<script setup lang="ts">
/**
 * 统一开关（role="switch"）。
 * 此前 SettingsDialog / SettingsMcpTab / PluginConfigForm / PluginsView 各自
 * 手写 role="switch" 按钮或 checkbox + 轨道样式，尺寸（40×22 / 34×20）与
 * knob 配色各不相同，现收敛为本组件：
 * - v-model 布尔；点击 / 空格 / 回车切换（原生 button 语义自带键盘激活）
 * - 选中底走 --accent、knob 走 --text-on-accent；未选中 knob 走 --text-3
 * - disabled 统一 --opacity-disabled（语义：控制项被锁）
 */
const props = withDefaults(
  defineProps<{
    modelValue: boolean
    disabled?: boolean
    /** sm = 34×20（紧凑行内），md = 40×22（设置行） */
    size?: 'sm' | 'md'
    /** 无可见文案时的可访问名（aria-label） */
    label?: string
  }>(),
  { disabled: false, size: 'md', label: '' },
)
const emit = defineEmits<{
  'update:modelValue': [value: boolean]
  change: [value: boolean]
}>()

function toggle(): void {
  if (props.disabled) return
  emit('update:modelValue', !props.modelValue)
  emit('change', !props.modelValue)
}
</script>

<template>
  <button
    type="button"
    role="switch"
    :aria-checked="modelValue"
    :aria-label="label || undefined"
    :disabled="disabled"
    class="sw"
    :class="[`sw-${size}`, { 'sw-on': modelValue }]"
    @click="toggle"
  >
    <span class="sw-knob" aria-hidden="true" />
  </button>
</template>

<style scoped>
.sw {
  position: relative;
  display: inline-flex;
  flex-shrink: 0;
  padding: 0;
  border-radius: var(--radius-pill);
  border: 1px solid var(--border-strong);
  background: var(--bg-active);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}
.sw:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
.sw:disabled {
  opacity: var(--opacity-disabled);
  cursor: default;
}
.sw-sm {
  width: 34px;
  height: 20px;
}
.sw-md {
  width: 40px;
  height: 22px;
}
.sw-on {
  background: var(--accent);
  border-color: var(--accent);
}
.sw-knob {
  position: absolute;
  top: 50%;
  left: 2px;
  transform: translateY(-50%);
  border-radius: 50%;
  background: var(--text-3);
  transition:
    transform var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.sw-sm .sw-knob {
  width: 14px;
  height: 14px;
}
.sw-md .sw-knob {
  width: 16px;
  height: 16px;
}
/* 选中：knob 平移到右侧留 2px 边距（sm: 32-14-2-2=14px；md: 38-16-2-2=18px） */
.sw-on .sw-knob {
  transform: translateY(-50%) translateX(14px);
  background: var(--text-on-accent);
}
.sw-md.sw-on .sw-knob {
  transform: translateY(-50%) translateX(18px);
}
</style>
