<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Compartment, EditorState } from '@codemirror/state'
import { Decoration, EditorView, highlightActiveLine, highlightActiveLineGutter, highlightSpecialChars, keymap, lineNumbers, placeholder } from '@codemirror/view'
import { closeBrackets, closeBracketsKeymap, autocompletion } from '@codemirror/autocomplete'
import { jsonCompletionSource } from '@/utils/jsonCompletion'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { javascript } from '@codemirror/lang-javascript'
import { json, jsonParseLinter } from '@codemirror/lang-json'
import { linter } from '@codemirror/lint'
import { bracketMatching, defaultHighlightStyle, foldGutter, indentOnInput, syntaxHighlighting, HighlightStyle } from '@codemirror/language'
import { tags } from '@lezer/highlight'

const props = withDefaults(
  defineProps<{
    modelValue: string
    readonly?: boolean
    placeholderText?: string
    autofocus?: boolean
    /** 查找词：非空时实时高亮全部匹配（大小写不敏感），无需回车。 */
    query?: string
    /** 当前选中的匹配索引（0-based，高亮加深）。 */
    activeMatch?: number
    /** 语言：json（默认，带实时 lint）/ javascript。 */
    language?: 'json' | 'javascript'
  }>(),
  {
    readonly: false,
    placeholderText: '',
    autofocus: false,
    query: '',
    activeMatch: 0,
    language: 'json',
  },
)

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const host = ref<HTMLElement | null>(null)
let view: EditorView | null = null
const readOnlyCompartment = new Compartment()
const searchCompartment = new Compartment()

/**
 * 语法色唯一真源是 style.css 的 --tok-*（深浅各一套，随 data-theme 联动），
 * 这里只引用 var()：同一份 JSON 在请求编辑器与响应树两侧颜色一致，
 * 组件内不再维护第二套硬编码色（表面主题同规则，见 cmTheme）。
 */
const syntaxHighlight = HighlightStyle.define([
  { tag: [tags.propertyName], color: 'var(--tok-key)' },
  { tag: [tags.string], color: 'var(--tok-str)' },
  { tag: [tags.number], color: 'var(--tok-num)' },
  { tag: [tags.bool], color: 'var(--tok-bool)' },
  { tag: [tags.null], color: 'var(--tok-null)' },
  { tag: [tags.punctuation, tags.bracket, tags.brace], color: 'var(--tok-punct)' },
  { tag: [tags.invalid], color: 'var(--danger)' },
  { tag: [tags.lineComment], color: 'var(--tok-gutter)' },
])

/**
 * 编辑器表面主题：单套 CSS 变量驱动（不再区分 dark/light 两份硬编码主题）。
 *
 * - 色源全部走主题 token：--code-*（表面/文字/光标）、--tok-*（行号）、
 *   --accent（选区）、--bg-hover（活动行）、--bg-elevated/--border-strong（tooltip）、
 *   --warning/--orange（搜索匹配）——深浅主题、8 套色板、自定义主题自动跟随；
 * - 主题切换 = CSS 变量换值，CodeMirror 无需 reconfigure / 重新初始化
 *   （不再监听 miyo:theme；themeCompartment 已随双主题一并移除）；
 * - 浅色下 --code-bg 是显式浅底，编辑器任何包裹处都不会退回深色残留。
 */
const cmTheme = EditorView.theme({
  '&': {
    height: '100%',
    fontSize: 'var(--fs-xs)',
    color: 'var(--code-fg)',
    backgroundColor: 'var(--code-bg)',
  },
  '.cm-scroller': {
    fontFamily: 'var(--font-mono)',
    lineHeight: '1.6',
  },
  '.cm-content': {
    padding: '8px 0',
    caretColor: 'var(--code-caret)',
  },
  '.cm-line': {
    padding: '0 8px',
  },
  '.cm-gutters': {
    backgroundColor: 'transparent',
    borderRight: '1px solid var(--border-editor)',
    color: 'var(--tok-gutter)',
  },
  '.cm-foldGutter .cm-gutterElement': {
    cursor: 'pointer',
  },
  '&.cm-focused': {
    outline: 'none',
  },
  '.cm-cursor': {
    borderLeftColor: 'var(--code-caret)',
  },
  '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, ::selection':
    {
      backgroundColor: 'color-mix(in srgb, var(--accent) 25%, transparent) !important',
    },
  '.cm-activeLine': {
    backgroundColor: 'var(--bg-hover)',
  },
  '.cm-activeLineGutter': {
    backgroundColor: 'var(--bg-hover)',
  },
  '.cm-tooltip': {
    backgroundColor: 'var(--bg-elevated)',
    border: '1px solid var(--border-strong)',
    color: 'var(--text-1)',
  },
  '.cm-tooltip-lint': {
    fontSize: 'var(--fs-xs)',
  },
  /* 覆盖 @codemirror/view 库内置的 .cm-placeholder 硬编码 #757575（3.7:1）→ token */
  '.cm-placeholder': {
    color: 'var(--code-placeholder)',
  },
  '.cm-foldPlaceholder': {
    backgroundColor: 'var(--bg-hover)',
    color: 'var(--text-3)',
    border: 'none',
  },
  '.cm-searchMatch': {
    backgroundColor: 'color-mix(in srgb, var(--warning) 22%, transparent)',
  },
  '.cm-searchMatchActive': {
    backgroundColor: 'color-mix(in srgb, var(--orange) 45%, transparent)',
    outline: '1px solid color-mix(in srgb, var(--orange) 80%, transparent)',
  },
})

/** 搜索高亮 mark：普通匹配淡黄底，当前匹配额外加深（两类同带，便于统一查询与样式覆盖）。 */
const searchMark = Decoration.mark({ class: 'cm-searchMatch' })
const searchActiveMark = Decoration.mark({ class: 'cm-searchMatch cm-searchMatchActive' })

/** 按 query 计算全文匹配装饰（大小写不敏感；空 query 返回空集）。 */
function searchDecorations(docText: string, query: string, active: number) {
  if (!query) return Decoration.none
  const lower = docText.toLowerCase()
  const ql = query.toLowerCase()
  const marks: { from: number; to: number; value: Decoration }[] = []
  let from = 0
  let idx = 0
  for (;;) {
    const pos = lower.indexOf(ql, from)
    if (pos === -1) break
    marks.push({ from: pos, to: pos + query.length, value: idx === active ? searchActiveMark : searchMark })
    idx += 1
    from = pos + ql.length
  }
  return Decoration.set(marks)
}

/** 按当前文档 + 查找词刷新高亮（无 query 时不 dispatch，保持零开销）。 */
function refreshSearchDecorations(): void {
  if (!view || !props.query) return
  view.dispatch({
    effects: searchCompartment.reconfigure(
      EditorView.decorations.of(
        searchDecorations(view.state.doc.toString(), props.query, props.activeMatch),
      ),
    ),
  })
}

/** 关闭查找时清掉残留高亮。 */
function clearSearchDecorations(): void {
  if (!view) return
  view.dispatch({
    effects: searchCompartment.reconfigure(EditorView.decorations.of(Decoration.none)),
  })
}
/** 编辑器扩展集（表面主题 + 语法高亮；两者均为 CSS 变量驱动，随主题即时联动）。 */
const cmThemeExtension = [cmTheme, syntaxHighlighting(syntaxHighlight)]

/**
 * 大文档降级：超 200k 字符时跳过实时 JSON lint（每次变更全量 parse），
 * 与 JsonEditor 的 LARGE_DOC_CHARS 对齐；Lezer 增量高亮本身保留。
 */
const LARGE_DOC_CHARS = 200_000

onMounted(() => {
  if (!host.value) return
  const largeDoc = props.modelValue.length > LARGE_DOC_CHARS
  view = new EditorView({
    parent: host.value,
    doc: props.modelValue,
    extensions: [
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightSpecialChars(),
      history(),
      foldGutter(),
      indentOnInput(),
      syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
      bracketMatching(),
      closeBrackets(),
      highlightActiveLine(),
      keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap, indentWithTab]),
      props.language === 'javascript' ? javascript() : json(),
      // JSON 上下文补全（键=同文已出现键，值=true/false/null）；JS 模式不启用
      ...(props.language === 'json' ? [autocompletion({ override: [jsonCompletionSource] })] : []),
      cmThemeExtension,
      // 大文档跳过实时 lint（见 LARGE_DOC_CHARS 说明）；JS 无 JSON lint。
      ...(props.language === 'javascript' || largeDoc ? [] : [linter(jsonParseLinter())]),
      readOnlyCompartment.of(EditorState.readOnly.of(props.readonly)),
      placeholder(props.placeholderText),
      searchCompartment.of(
        EditorView.decorations.of(
          searchDecorations(props.modelValue, props.query, props.activeMatch),
        ),
      ),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          scheduleEmit(update.state.doc.toString())
        }
      }),
    ],
  })
  if (props.autofocus) view.focus()
})

watch(
  () => props.modelValue,
  (val) => {
    if (!view) return
    const current = view.state.doc.toString()
    if (current !== val) {
      view.dispatch({ changes: { from: 0, to: current.length, insert: val } })
    }
    // 外部回写（如切换用例）后按新文档刷新高亮。
    if (props.query) refreshSearchDecorations()
  },
)

/** 查找词/当前匹配变化 → 实时刷新高亮（输入即标，无需回车）。 */
watch(
  [() => props.query, () => props.activeMatch],
  ([q]) => {
    if (!view) return
    if (q) refreshSearchDecorations()
    else clearSearchDecorations()
  },
)

watch(
  () => props.readonly,
  (val) => {
    view?.dispatch({ effects: readOnlyCompartment.reconfigure(EditorState.readOnly.of(val)) })
  },
)

/**
 * 回写防抖：每键即时 emit 会经父级 v-model 触发全文档回写检查
 *（`modelValue` watcher 做全量替换，大 JSON 下光标跳 + 全量重解析）。
 * 编辑态保留在 CodeMirror 本地（Lezer 增量解析本来就快），120ms 后
 * 一次性同步给父级；卸载前强制刷出 pending 值，避免"键入即关"丢数据。
 */
let emitTimer: ReturnType<typeof setTimeout> | undefined
let pendingEmit: string | null = null
function scheduleEmit(value: string): void {
  pendingEmit = value
  if (emitTimer) clearTimeout(emitTimer)
  emitTimer = setTimeout(() => {
    emitTimer = undefined
    if (pendingEmit !== null) {
      emit('update:modelValue', pendingEmit)
      pendingEmit = null
    }
  }, 120)
}
function flushEmit(): void {
  if (emitTimer) {
    clearTimeout(emitTimer)
    emitTimer = undefined
  }
  if (pendingEmit !== null) {
    emit('update:modelValue', pendingEmit)
    pendingEmit = null
  }
}

function requestMeasure(): void {
  view?.requestMeasure()
}

/**
 * 选中第 index 个匹配（大小写不敏感），并滚动到可见区。
 * 供 FindBar 上一个/下一个跳转用；无匹配时不做任何事。
 * 注意：滚动与聚焦会触发 CodeMirror 的异步布局测量，jsdom 等
 * 无布局环境缺少 Range#getClientRects——此时仅选中不滚动/聚焦。
 */
function selectMatch(query: string, index: number): void {
  if (!view || !query) return
  const doc = view.state.doc.toString()
  const lower = doc.toLowerCase()
  const ql = query.toLowerCase()
  let from = 0
  let cur = 0
  for (;;) {
    const idx = lower.indexOf(ql, from)
    if (idx === -1) return
    if (cur === index) {
      view.dispatch({
        selection: { anchor: idx, head: idx + query.length },
      })
      if (typeof document.createRange().getClientRects === 'function') {
        view.dispatch({ scrollIntoView: true })
        view.focus()
      }
      return
    }
    cur += 1
    from = idx + ql.length
  }
}

defineExpose({ requestMeasure, selectMatch, focus: () => view?.focus() })

onBeforeUnmount(() => {
  flushEmit()
  view?.destroy()
  view = null
})
</script>

<template>
  <div ref="host" class="cm-host"></div>
</template>

<style scoped>
.cm-host {
  height: 100%;
  min-height: 0;
  overflow: hidden;
}
.cm-host :deep(.cm-editor) {
  height: 100%;
}
.cm-host :deep(.cm-scroller) {
  overflow: auto;
}
</style>