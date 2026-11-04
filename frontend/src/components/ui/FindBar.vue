<script setup lang="ts">
/**
 * FindBar：文本内查找/替换条（响应查看器与请求 Body 共用）。
 * 输入即搜；Enter = 下一个，Shift + Enter = 上一个，Esc = 关闭。
 * 打开时自动聚焦输入框。
 * 替换行为可选项（replaceable）：不传时与旧版纯查找条完全一致。
 * 替换行遵循一般编辑器惯例：替换输入框 Enter = 替换下一处（单个替换），
 * Ctrl/Cmd+Alt+Enter = 全部替换，Esc = 关闭整个查找条。
 */
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import Icon from './Icon.vue'

const locale = useLocaleStore()
const t = locale.t

const props = withDefaults(
  defineProps<{
    query: string
    index: number
    total: number
    placeholder?: string
    /** 是否提供替换行（请求 Body 编辑器开启；响应查看器等保持纯查找）。 */
    replaceable?: boolean
    /** 替换行是否展开（宿主持有状态，快捷键可直接展开）。 */
    replaceOpen?: boolean
    /** 替换内容（v-model:replace）。 */
    replace?: string
    replacePh?: string
  }>(),
  { placeholder: '', replaceable: false, replaceOpen: false, replace: '', replacePh: '' },
)

/** 未传 placeholder 时按当前语言兜底。 */
const effectivePlaceholder = computed(() => props.placeholder || t('findbar.ph'))
const effectiveReplacePh = computed(() => props.replacePh || t('findbar.replacePh'))

const emit = defineEmits<{
  'update:query': [string]
  'update:replace': [string]
  'toggle-replace': []
  'replace-one': []
  'replace-all': []
  prev: []
  next: []
  close: []
}>()

/** 输入值代理：写入时上报给父级（响应式 v-model 语义）。 */
const inputValue = computed({
  get: () => props.query,
  set: (v: string) => emit('update:query', v),
})
const replaceValue = computed({
  get: () => props.replace,
  set: (v: string) => emit('update:replace', v),
})

const input = ref<HTMLInputElement | null>(null)
const replaceInput = ref<HTMLInputElement | null>(null)

onMounted(() => {
  // 快捷键直接展开替换行打开时，焦点落在替换输入框；否则聚焦查找框。
  if (props.replaceable && props.replaceOpen) replaceInput.value?.focus()
  else input.value?.focus()
})

// 已打开的查找条上展开替换行（展开按钮 / 快捷键）→ 焦点跟进替换输入框。
watch(
  () => props.replaceOpen,
  (open) => {
    if (open) void nextTick(() => replaceInput.value?.focus())
  },
)

function onInputKeydown(e: KeyboardEvent): void {
  if (e.key === 'Enter') {
    e.preventDefault()
    if (e.shiftKey) emit('prev')
    else emit('next')
  } else if (e.key === 'Escape') {
    e.preventDefault()
    emit('close')
  }
}

function onReplaceKeydown(e: KeyboardEvent): void {
  if (e.key === 'Enter') {
    e.preventDefault()
    // 一般编辑器惯例：Enter 替换下一处；Ctrl/Cmd+Alt+Enter 全部替换
    if (e.metaKey || e.ctrlKey || e.altKey) emit('replace-all')
    else emit('replace-one')
  } else if (e.key === 'Escape') {
    e.preventDefault()
    emit('close')
  }
}
</script>

<template>
  <div class="findbar-wrap">
    <div class="findbar">
      <button
        v-if="replaceable"
        class="findbar-btn findbar-expander"
        :class="{ open: replaceOpen }"
        type="button"
        :title="t('findbar.toggleReplace')"
        :aria-label="t('findbar.toggleReplace')"
        :aria-expanded="replaceOpen"
        @click="emit('toggle-replace')"
      >
        <Icon name="chevron-right" :size="13" />
      </button>
      <Icon name="search" :size="13" class="findbar-icon" />
      <input
        ref="input"
        v-model="inputValue"
        class="findbar-input"
        spellcheck="false"
        :placeholder="effectivePlaceholder"
        @keydown="onInputKeydown"
      />
      <span class="findbar-count" :class="{ empty: total === 0 }">
        {{ total ? `${index + 1} / ${total}` : t('findbar.noMatch') }}
      </span>
      <button
        class="findbar-btn"
        type="button"
        :title="t('findbar.prev')"
        :aria-label="t('findbar.prev')"
        :disabled="total === 0"
        @click="emit('prev')"
      >
        <Icon name="chevron-up" :size="13" />
      </button>
      <button
        class="findbar-btn"
        type="button"
        :title="t('findbar.next')"
        :aria-label="t('findbar.next')"
        :disabled="total === 0"
        @click="emit('next')"
      >
        <Icon name="chevron-down" :size="13" />
      </button>
      <button
        class="findbar-btn findbar-close"
        type="button"
        :title="t('findbar.close')"
        :aria-label="t('findbar.close')"
        @click="emit('close')"
      >
        <Icon name="x" :size="13" />
      </button>
    </div>
    <div v-if="replaceable && replaceOpen" class="findbar findbar-replace-row">
      <span class="findbar-row-spacer" aria-hidden="true"></span>
      <input
        ref="replaceInput"
        v-model="replaceValue"
        class="findbar-input"
        spellcheck="false"
        :placeholder="effectiveReplacePh"
        @keydown="onReplaceKeydown"
      />
      <button
        class="findbar-replace-btn"
        type="button"
        :title="t('findbar.replaceOneHint')"
        :aria-label="t('findbar.replaceOneHint')"
        :disabled="!query"
        @click="emit('replace-one')"
      >
        {{ t('findbar.replaceOne') }}
      </button>
      <button
        class="findbar-replace-btn"
        type="button"
        :title="t('findbar.replaceAllHint')"
        :aria-label="t('findbar.replaceAllHint')"
        :disabled="!query"
        @click="emit('replace-all')"
      >
        {{ t('findbar.replaceAll') }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.findbar-wrap {
  border-bottom: 1px solid var(--border);
  background: var(--bg-panel);
}

.findbar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
}

/* 替换行与查找行的输入框左缘对齐（展开钮 24 + gap 6 + 图标 13 + gap 6）。 */
.findbar-replace-row {
  padding-top: 0;
}
.findbar-row-spacer {
  flex-shrink: 0;
  width: 49px;
}

.findbar-icon {
  flex-shrink: 0;
  color: var(--text-3);
}

.findbar-input {
  flex: 1 1 auto;
  min-width: 0;
  padding: 4px 8px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-1);
  outline: none;
}
.findbar-input:focus {
  border-color: var(--accent);
}

.findbar-count {
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-2);
  min-width: 52px;
  text-align: right;
}
.findbar-count.empty {
  color: var(--text-3);
}

.findbar-btn {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: none;
  border-radius: 6px;
  background: none;
  color: var(--text-2);
  cursor: pointer;
}
.findbar-btn:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-1);
}
.findbar-btn:disabled {
  opacity: 0.4;
  cursor: default;
}

.findbar-close:hover {
  color: var(--danger);
}

/* 展开钮：关闭时朝右（展开替换行），展开时转 90° 朝下（收起）。 */
.findbar-expander {
  transition: transform var(--dur) var(--ease);
}
.findbar-expander.open {
  transform: rotate(90deg);
}

.findbar-replace-btn {
  flex-shrink: 0;
  padding: 4px 10px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  color: var(--text-1);
  font-size: var(--fs-xxs);
  cursor: pointer;
  transition:
    border-color var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.findbar-replace-btn:hover:not(:disabled) {
  border-color: var(--accent);
  color: var(--accent);
}
.findbar-replace-btn:disabled {
  opacity: 0.4;
  cursor: default;
}
</style>
