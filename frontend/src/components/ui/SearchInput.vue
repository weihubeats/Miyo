<script setup lang="ts">
/**
 * SearchInput：统一搜索框（图标 + 输入 + 可清空）。
 *
 * 全仓唯一搜索框实现，替代此前 10+ 处自绘「search 图标 + input + 清除钮」组合
 * （HistoryPanel / CookiePanel / EnvVarTable / EnvironmentSelector / FrameLog /
 * TestCasesPanel / ScenarioLibrary / ScenarioVarsPanel / WorkspaceView /
 * ProjectList / JavaClassBrowser / PluginsView …）。
 *
 * - v-model 绑定关键字；Enter 触发 search 事件（即时过滤的页面可不监听）；
 * - 有值时显示清除钮（Esc 也可清空），键盘可达（按钮可聚焦）；
 * - append 插槽：框内尾部同级操作（如状态过滤下拉），与过滤条同一工具栏；
 * - 视觉走全局 token（--h-sm 井底 + --border 描边 + focus 光环），深浅主题自动跟随。
 */
import { ref } from 'vue'
import Icon from './Icon.vue'
import { useLocaleStore } from '@/stores/locale'

const props = withDefaults(
  defineProps<{
    modelValue: string
    placeholder?: string
    /** md = 控件标准高（--h-md），默认 sm = 工具条高（--h-sm）。 */
    size?: 'sm' | 'md'
    disabled?: boolean
  }>(),
  { placeholder: '', size: 'sm', disabled: false },
)

const emit = defineEmits<{
  'update:modelValue': [value: string]
  /** Enter 提交（即时过滤场景可不监听）。 */
  search: []
  /** 输入框获得焦点（父级可做快捷键抑制等）。 */
  focus: []
  /** Esc 清空时通知（父级默认过滤逻辑之外的特殊处理用）。 */
  clear: []
}>()

const locale = useLocaleStore()
const t = locale.t

const focused = ref(false)
const inputEl = ref<HTMLInputElement | null>(null)

function onInput(e: Event): void {
  emit('update:modelValue', (e.target as HTMLInputElement).value)
}

function onKeydown(e: KeyboardEvent): void {
  if (e.key === 'Enter') emit('search')
  if (e.key === 'Escape' && props.modelValue) {
    e.stopPropagation()
    emit('update:modelValue', '')
    emit('clear')
  }
}

function clear(): void {
  emit('update:modelValue', '')
  emit('clear')
}

/** 聚焦输入框（⌘K 等全局快捷键调用）。 */
function focus(): void {
  inputEl.value?.focus()
}

/** 全选当前关键字（focus 后紧跟调用，方便直接替换输入）。 */
function select(): void {
  inputEl.value?.select()
}

defineExpose({ focus, select })
</script>

<template>
  <div class="si" :class="[`si-${size}`, { 'si-focused': focused, 'si-disabled': disabled }]">
    <Icon name="search" :size="12" class="si-icon" aria-hidden="true" />
    <input
      ref="inputEl"
      class="si-input"
      type="text"
      spellcheck="false"
      :value="modelValue"
      :placeholder="placeholder"
      :disabled="disabled"
      :aria-label="placeholder || t('common.search')"
      @input="onInput"
      @keydown="onKeydown"
      @focus="focused = true; emit('focus')"
      @blur="focused = false"
    />
    <slot name="append" />
    <button
      v-if="modelValue && !disabled"
      type="button"
      class="si-clear"
      :title="t('common.clearSearch')"
      :aria-label="t('common.clearSearch')"
      @click="clear"
    >
      <Icon name="x" :size="11" aria-hidden="true" />
    </button>
  </div>
</template>

<style scoped>
.si {
  display: flex;
  align-items: center;
  gap: 6px;
  height: var(--h-sm);
  padding: 0 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-code);
  transition:
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.si:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint);
}
.si-disabled {
  opacity: 0.5;
}
.si-md {
  height: var(--h-md);
}

.si-icon {
  flex-shrink: 0;
  color: var(--text-3);
}

.si-input {
  flex: 1;
  min-width: 0;
  height: 100%;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-1);
  font-size: var(--fs-xs);
  font-family: var(--font-mono);
  outline: none;
}
.si-input::placeholder {
  color: var(--text-3);
}

.si-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  width: 18px;
  height: 18px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
  transition: background var(--dur) var(--ease);
}
.si-clear:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
</style>
