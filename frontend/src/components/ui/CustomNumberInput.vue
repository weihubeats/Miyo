<script setup lang="ts">
/**
 * CustomNumberInput：自绘数字输入（步进按钮 + 键盘 ↑/↓/Enter + min/max 钳制）。
 * - 聚焦期间维护编辑缓冲 edit，失焦以可解析的输入内容提交——父级只监听
 *   change（单向绑定）也能手输生效（#134）；双向回显父级行为不变；
 * - 输出保持 number（可解析时）或原始字符串（编辑中途）；
 * - 五态：default / hover / focus / active / disabled × 深/浅主题；tabular-nums。
 */
import { computed, ref } from 'vue'
import Icon from './Icon.vue'
import { useLocaleStore } from '../../stores/locale'

const props = withDefaults(
  defineProps<{
    modelValue?: string | number | null
    min?: number
    max?: number
    step?: number
    placeholder?: string
    disabled?: boolean
    size?: 'sm' | 'md'
    /** 输入框视觉基调：default 用主题卡片色；inset 用深陷半透明底（适合设置面板）。 */
    tone?: 'default' | 'inset'
  }>(),
  { modelValue: '', min: undefined, max: undefined, step: 1, placeholder: '', disabled: false, size: 'md', tone: 'default' },
)

const t = useLocaleStore().t

const emit = defineEmits<{
  'update:modelValue': [value: string | number]
  change: [value: number]
}>()

/** 聚焦期间的编辑缓冲：单向绑定父级（只监听 change）不回显 prop，也以输入内容为准。 */
const focused = ref(false)
const edit = ref('')

const text = computed(() => (focused.value ? edit.value : String(props.modelValue ?? '')))

function parse(input: string): number | null {
  if (!input.trim()) return null
  const n = Number(input)
  return Number.isFinite(n) ? n : null
}

function clamp(n: number): number {
  let v = n
  if (props.min !== undefined) v = Math.max(props.min, v)
  if (props.max !== undefined) v = Math.min(props.max, v)
  return v
}

function commit(v: number | null): void {
  const raw = props.modelValue
  if (v !== null) {
    edit.value = String(v)
    emit('update:modelValue', v)
    emit('change', v)
  } else if (String(raw) !== '') {
    edit.value = ''
    emit('update:modelValue', '')
  }
}

function onFocus(): void {
  focused.value = true
  edit.value = String(props.modelValue ?? '')
}

function onInput(event: Event): void {
  const raw = (event.target as HTMLInputElement).value
  edit.value = raw
  const n = parse(raw)
  if (n !== null) emit('update:modelValue', n)
  else emit('update:modelValue', raw)
}

function stepBy(delta: number): void {
  const base = parse(text.value)
  const next = base === null ? (props.min ?? 0) : base + delta * props.step
  commit(clamp(next))
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'ArrowUp') {
    event.preventDefault()
    stepBy(1)
  } else if (event.key === 'ArrowDown') {
    event.preventDefault()
    stepBy(-1)
  } else if (event.key === 'Enter') {
    // 手输后回车直接走失焦提交路径
    ;(event.target as HTMLInputElement).blur()
  }
}

function onBlur(): void {
  focused.value = false
  const baseline = String(props.modelValue ?? '')
  // 手输过且可解析 → 以输入内容为准（父级未回显的单向绑定也能提交）；
  // 否则维持原语义按父级值判定：可解析则钳制提交，非法/空输入不误写回。
  const edited = edit.value !== baseline
  const source = edited && parse(edit.value) !== null ? edit.value : baseline
  const n = parse(source)
  if (n === null) {
    if (source.trim()) emit('update:modelValue', '')
  } else {
    commit(clamp(n))
  }
}
</script>

<template>
  <span class="cni" :class="[`size-${size}`, `tone-${tone}`, { disabled }]">
    <input
      class="cni-input"
      type="text"
      inputmode="numeric"
      :value="text"
      :placeholder="placeholder"
      :disabled="disabled"
      spellcheck="false"
      @focus="onFocus"
      @input="onInput"
      @keydown="onKeydown"
      @blur="onBlur"
    />
    <span class="cni-steppers">
      <button type="button" class="cni-step" tabindex="-1" :aria-label="t('cnum.increase')" :disabled="disabled" @mousedown.prevent @click="stepBy(1)">
        <Icon name="chevron-up" :size="10" />
      </button>
      <button type="button" class="cni-step" tabindex="-1" :aria-label="t('cnum.decrease')" :disabled="disabled" @mousedown.prevent @click="stepBy(-1)">
        <Icon name="chevron-down" :size="10" />
      </button>
    </span>
  </span>
</template>

<style scoped>
.cni {
  display: inline-flex;
  align-items: center;
  position: relative;
  flex: 0 0 auto;
}
.cni.size-md .cni-input {
  height: var(--h-md);
  font-size: 13px;
}
.cni.size-sm .cni-input {
  height: var(--h-sm);
  font-size: 12px;
}

.cni-input {
  width: 100%;
  border: 1px solid var(--border-strong);
  background: var(--bg-card);
  color: var(--text-1);
  border-radius: var(--radius);
  padding: 0 26px 0 10px;
  font-family: var(--font-mono);
  font-size: 12.5px;
  font-variant-numeric: tabular-nums;
  outline: none;
  transition:
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.cni-input::placeholder {
  color: var(--text-3);
}
.cni-input:hover:not(:disabled) {
  border-color: var(--border-strong);
}
.cni-input:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint);
}
.cni.disabled .cni-input {
  opacity: 0.45;
  cursor: default;
}

/* ---- inset 基调：深陷半透明底（设置面板用）；双主题值走 --inset-input-* ---- */
.cni.tone-inset .cni-input {
  background: var(--inset-input-bg);
  border-color: var(--inset-input-border);
}
.cni.tone-inset .cni-input:hover:not(:disabled) {
  border-color: var(--inset-input-border-hover);
}
.cni.tone-inset .cni-input:focus {
  border-color: rgba(168, 85, 247, 0.5);
  box-shadow: 0 0 0 2px rgba(168, 85, 247, 0.12);
}
.cni.tone-inset .cni-step:hover:not(:disabled) {
  color: var(--text-2);
}

.cni-steppers {
  position: absolute;
  right: 3px;
  top: 50%;
  transform: translateY(-50%);
  display: flex;
  flex-direction: column;
  pointer-events: none;
}
.cni-step {
  pointer-events: auto;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 11px;
  border: none;
  background: transparent;
  color: var(--text-3);
  border-radius: var(--radius-sm);
  cursor: pointer;
  padding: 0;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.cni-step:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-1);
}
.cni-step:active:not(:disabled) {
  background: var(--accent-tint);
  color: var(--accent);
}
.cni-step:disabled {
  opacity: 0.4;
  cursor: default;
}
</style>
