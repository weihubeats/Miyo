<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Compartment, EditorState } from '@codemirror/state'
import { Decoration, EditorView, highlightActiveLine, highlightActiveLineGutter, highlightSpecialChars, keymap, lineNumbers, placeholder } from '@codemirror/view'
import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { javascript } from '@codemirror/lang-javascript'
import { json, jsonParseLinter } from '@codemirror/lang-json'
import { linter } from '@codemirror/lint'
import { bracketMatching, defaultHighlightStyle, foldGutter, indentOnInput, syntaxHighlighting, HighlightStyle } from '@codemirror/language'
import { tags } from '@lezer/highlight'
import { THEME_EVENT } from '../stores/theme'

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
const themeCompartment = new Compartment()
const searchCompartment = new Compartment()

/** 当前主题：以 <html> 的 data-theme 为准，并监听 miyo:theme 全局事件联动。 */
const theme = ref<'dark' | 'light'>(readThemeFromDom())
function readThemeFromDom(): 'dark' | 'light' {
  return document.documentElement.getAttribute('data-theme') === 'light' ? 'light' : 'dark'
}
function onThemeEvent(e: Event): void {
  const mode = (e as CustomEvent<{ mode: 'dark' | 'light' }>).detail?.mode
  if (mode === 'dark' || mode === 'light') theme.value = mode
}

/**
 * 语法色唯一真源是 style.css 的 --tok-*（深浅各一套，随 data-theme 联动），
 * 这里只引用 var()：同一份 JSON 在请求编辑器与响应树两侧颜色一致，
 * 组件内不再维护第二套硬编码色。
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

const darkTheme = EditorView.theme({
  '&': {
    height: '100%',
    fontSize: '12px',
    color: '#e2e8f0',
  },
  '.cm-scroller': {
    fontFamily: 'var(--font-mono)',
    lineHeight: '1.6',
  },
  '.cm-content': {
    padding: '8px 0',
    caretColor: '#a78bfa',
  },
  '.cm-line': {
    padding: '0 8px',
  },
  '.cm-gutters': {
    backgroundColor: 'transparent',
    borderRight: '1px solid rgba(148, 163, 184, 0.15)',
    color: '#64748b',
  },
  '.cm-foldGutter .cm-gutterElement': {
    cursor: 'pointer',
  },
  '&.cm-focused': {
    outline: 'none',
  },
  '.cm-cursor': {
    borderLeftColor: '#a78bfa',
  },
  '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, ::selection':
    {
      backgroundColor: 'rgba(168, 85, 247, 0.25) !important',
    },
  '.cm-activeLine': {
    backgroundColor: 'rgba(148, 163, 184, 0.06)',
  },
  '.cm-activeLineGutter': {
    backgroundColor: 'rgba(148, 163, 184, 0.06)',
  },
  '.cm-tooltip': {
    backgroundColor: '#1e293b',
    border: '1px solid #334155',
    color: '#e2e8f0',
  },
  '.cm-tooltip-lint': {
    fontSize: '12px',
  },
  '.cm-foldPlaceholder': {
    backgroundColor: 'rgba(148, 163, 184, 0.15)',
    color: '#94a3b8',
    border: 'none',
  },
  '.cm-searchMatch': {
    backgroundColor: 'rgba(251, 191, 36, 0.25)',
  },
  '.cm-searchMatchActive': {
    backgroundColor: 'rgba(251, 146, 60, 0.55)',
    outline: '1px solid rgba(251, 146, 60, 0.9)',
  },
})

const lightTheme = EditorView.theme({
  '&': {
    height: '100%',
    fontSize: '12px',
    color: '#1f2329',
    // 显式浅底（slate-50）：不依赖容器透传，任何包裹处都不会退回深色残留
    backgroundColor: '#f8fafc',
  },
  '.cm-scroller': {
    fontFamily: 'var(--font-mono)',
    lineHeight: '1.6',
  },
  '.cm-content': {
    padding: '8px 0',
    caretColor: '#7c3aed',
  },
  '.cm-line': {
    padding: '0 8px',
  },
  '.cm-gutters': {
    backgroundColor: 'transparent',
    borderRight: '1px solid rgba(107, 114, 128, 0.2)',
    color: '#94a3b8',
  },
  '.cm-foldGutter .cm-gutterElement': {
    cursor: 'pointer',
  },
  '&.cm-focused': {
    outline: 'none',
  },
  '.cm-cursor': {
    borderLeftColor: '#7c3aed',
  },
  '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, ::selection':
    {
      backgroundColor: 'rgba(124, 58, 237, 0.2) !important',
    },
  '.cm-activeLine': {
    backgroundColor: 'rgba(107, 114, 128, 0.08)',
  },
  '.cm-activeLineGutter': {
    backgroundColor: 'rgba(107, 114, 128, 0.08)',
  },
  '.cm-tooltip': {
    backgroundColor: '#ffffff',
    border: '1px solid #d0d5dd',
    color: '#1f2329',
  },
  '.cm-tooltip-lint': {
    fontSize: '12px',
  },
  '.cm-foldPlaceholder': {
    backgroundColor: 'rgba(107, 114, 128, 0.12)',
    color: '#6b7280',
    border: 'none',
  },
  '.cm-searchMatch': {
    backgroundColor: 'rgba(217, 119, 6, 0.2)',
  },
  '.cm-searchMatchActive': {
    backgroundColor: 'rgba(249, 115, 22, 0.35)',
    outline: '1px solid rgba(249, 115, 22, 0.8)',
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
/** 当前生效主题对应的扩展集（主题样式 + 语法高亮；高亮本身走 --tok-* 随主题联动）。 */
function currentThemeExtension() {
  return [theme.value === 'dark' ? darkTheme : lightTheme, syntaxHighlighting(syntaxHighlight)]
}

/**
 * 大文档降级：超 200k 字符时跳过实时 JSON lint（每次变更全量 parse），
 * 与 JsonEditor 的 LARGE_DOC_CHARS 对齐；Lezer 增量高亮本身保留。
 */
const LARGE_DOC_CHARS = 200_000

onMounted(() => {
  window.addEventListener(THEME_EVENT, onThemeEvent)
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
      themeCompartment.of(currentThemeExtension()),
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
  theme,
  () => {
    if (!view) return
    view.dispatch({ effects: themeCompartment.reconfigure(currentThemeExtension()) })
  },
)

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
  () => [props.query, props.activeMatch] as const,
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
  window.removeEventListener(THEME_EVENT, onThemeEvent)
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