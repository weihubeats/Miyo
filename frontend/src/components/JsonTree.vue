<script setup lang="ts">
/**
 * JsonTree：JSON 树形查看器（响应体 Pretty 视图）。
 *
 * - 扁平行渲染：每行带行号 + 缩进 + 折叠箭头，容器节点（对象/数组）可折叠/展开；
 * - 语法着色：键 / 字符串 / 数字 / 布尔 / null / 标点；
 * - 默认全展开（expandDepth 很大），超大响应由 maxLines 渲染上限兜底；
 * - 长字符串截断展示，悬浮显示全文；
 * - 查找（Find in Response）：`query` + `activeMatch` 驱动，高亮全部匹配、
 *   当前匹配加亮并滚动到可视区；搜索时强制展开所有节点以保证匹配完整；
 *   匹配总数通过 `match-count` 事件上报；
 * - 外部可通过 `expandAll()` / `collapseAll()` 控制全部节点的展开状态。
 */
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { escapeHtml } from '../utils/highlight'
import { useLocaleStore } from '../stores/locale'
import Icon from './ui/Icon.vue'

/** 语法片段：文本 + 着色类名。 */
interface Seg {
  text: string
  cls: string
  title?: string
}

interface Line {
  depth: number
  segments: Seg[]
  title?: string
  toggleable?: string
  open?: boolean
}

/** 一行内的匹配范围（字符偏移，相对该行文本）。 */
type Range = [number, number]

const props = withDefaults(
  defineProps<{
    data: unknown
    expandDepth?: number
    /** 查找词（非空时强制展开全部节点并高亮）。 */
    query?: string
    /** 当前激活的匹配序号（0-based，由父级控制上/下一个）。 */
    activeMatch?: number
    /** 渲染行数上限：展开全部 / 查找强制展开时，避免一次渲染数万行 DOM 冻结页面。 */
    maxLines?: number
  }>(),
  // 3000 行：万行级 DOM 挂载会让大响应首帧明显卡顿（原默认 10000）。
  // 超出时末行给出提示，引导切到响应区「原始」行视图分页查看全部内容。
  { expandDepth: 99, query: '', activeMatch: 0, maxLines: 3000 },
)

const emit = defineEmits<{ 'match-count': [number] }>()

const locale = useLocaleStore()
const t = locale.t

/** 折叠状态：path（`$["key"]` / `$[0]`）→ 是否展开。 */
const expanded = reactive<Record<string, boolean>>({})

/**
 * 全局展开标记：`expandAll` / `collapseAll` 原来递归收集全量容器 path
 * 并逐个写入 reactive，大 JSON 直接卡死。改为 O(1) 标记：
 * 非 null 时覆盖所有未显式切换节点的展开态，单节点 toggle 写入
 * `expanded` 覆盖标记（模板把当前行 open 态传进来，无需查 depth）。
 */
const forceOpen = ref<boolean | null>(null)

// 数据更换（新响应）时清空折叠状态：旧 path 键跨响应累积既持续占用内存，
// 也会让新响应中同路径节点意外保持展开。同一标签页内组件实例是复用的。
watch(
  () => props.data,
  () => {
    for (const k of Object.keys(expanded)) delete expanded[k]
    forceOpen.value = null
  },
)

const rootRef = ref<HTMLDivElement | null>(null)

function toggle(path: string, currentOpen: boolean): void {
  expanded[path] = !currentOpen
}

function tok(text: string, cls: string): Seg {
  return { text, cls }
}

function keyToken(key: string): Seg[] {
  return [tok(JSON.stringify(key), 'key'), tok(': ', 'punct')]
}

function leafSegs(value: unknown): Seg[] {
  if (typeof value === 'string') {
    const truncated = value.length > 160 ? `${value.slice(0, 160)}…` : value
    return [tok(JSON.stringify(truncated), 'str')]
  }
  if (typeof value === 'number') return [tok(String(value), 'num')]
  if (typeof value === 'boolean') return [tok(String(value), 'bool')]
  return [tok('null', 'null')]
}

const lines = computed<Line[]>(() => {
  const out: Line[] = []
  // 查找激活时强制展开：保证折叠节点内的文本也能被匹配到。
  const force = props.query.length > 0
  let capped = false

  function walk(
    value: unknown,
    depth: number,
    path: string,
    keyHtml: Seg[] | null,
    isLast: boolean,
  ): void {
    if (out.length >= props.maxLines) {
      capped = true
      return
    }
    if (value === null || typeof value !== 'object') {
      const segments = [...(keyHtml ?? []), ...leafSegs(value)]
      if (!isLast) segments.push(tok(',', 'punct'))
      out.push({ depth, segments, ...(typeof value === 'string' ? { title: value } : {}) })
      return
    }

    const isArray = Array.isArray(value)
    const count = isArray ? (value as unknown[]).length : Object.keys(value as object).length

    if (count === 0) {
      const segments = [...(keyHtml ?? []), tok(isArray ? '[]' : '{}', 'punct')]
      if (!isLast) segments.push(tok(',', 'punct'))
      out.push({ depth, segments })
      return
    }

    const open = force || (forceOpen.value ?? (expanded[path] ?? depth < props.expandDepth))
    if (!open) {
      const segments = [
        ...(keyHtml ?? []),
        tok(isArray ? '[' : '{', 'punct'),
        tok(' … ', 'dots'),
        tok(t('jsonTree.itemCount', { n: count }), 'meta'),
        tok(isArray ? ']' : '}', 'punct'),
      ]
      if (!isLast) segments.push(tok(',', 'punct'))
      out.push({ depth, segments, toggleable: path, open: false })
      return
    }

    const head: Seg[] = [...(keyHtml ?? []), tok(isArray ? '[' : '{', 'punct')]
    out.push({ depth, segments: head, toggleable: path, open: true })

    if (isArray) {
      const arr = value as unknown[]
      for (let i = 0; i < arr.length; i++) {
        walk(arr[i], depth + 1, `${path}[${i}]`, null, i === arr.length - 1)
      }
    } else {
      const entries = Object.entries(value as Record<string, unknown>)
      for (let i = 0; i < entries.length; i++) {
        const [k, v] = entries[i]
        walk(v, depth + 1, `${path}[${JSON.stringify(k)}]`, keyToken(k), i === entries.length - 1)
      }
    }

    const tail: Seg[] = [tok(isArray ? ']' : '}', 'punct')]
    if (!isLast) tail.push(tok(',', 'punct'))
    out.push({ depth, segments: tail })
  }

  if (props.data !== undefined) walk(props.data, 0, '$', null, true)
  if (capped) {
    out.push({
      depth: 0,
      segments: [
        {
          text: t('jsonTree.capped', { n: props.maxLines.toLocaleString() }),
          cls: 'meta',
        },
        tok(' · ', 'punct'),
        tok(t('jsonTree.cappedHint'), 'meta'),
      ],
    })
  }
  return out
})

// ---------- 查找 ----------
/** 每行 [行号 → 匹配范围数组]（保持行号升序）。 */
const matches = computed<Map<number, Range[]>>(() => {
  const map = new Map<number, Range[]>()
  const q = props.query
  if (!q) return map
  const ql = q.toLowerCase()
  lines.value.forEach((line, i) => {
    const text = line.segments.map((s) => s.text).join('')
    const lower = text.toLowerCase()
    const ranges: Range[] = []
    let from = 0
    for (;;) {
      const idx = lower.indexOf(ql, from)
      if (idx === -1) break
      ranges.push([idx, idx + q.length])
      from = idx + q.length
    }
    if (ranges.length) map.set(i, ranges)
  })
  return map
})

/** 匹配总数（供父级展示 n/N 与上/下一个导航）。 */
const matchCount = computed(() => {
  let n = 0
  for (const ranges of matches.value.values()) n += ranges.length
  return n
})

watch(matchCount, (n) => emit('match-count', n))
watch(
  () => props.query,
  () => emit('match-count', matchCount.value),
  { immediate: true },
)

/**
 * 每行的最终 HTML（含查找高亮）：一次遍历，同时推进全局匹配序号。
 *
 * 注意：不再依赖 `activeMatch`——上/下一个导航原来每次触发万行 HTML
 * 全量重建；现在 mark 只带序号，激活态由 watcher 做 DOM 级切换。
 */
const lineHtmls = computed(() => {
  const htmls: string[] = []
  let global = 0
  for (const line of lines.value) {
    if (!props.query) {
      htmls.push(line.segments.map((s) => tokHtml(s)).join(''))
      continue
    }
    const lineMatches = matches.value.get(htmls.length)
    htmls.push(renderHighlighted(line, lineMatches, global))
    global += lineMatches?.length ?? 0
  }
  return htmls
})

function tokHtml(seg: Seg): string {
  return `<span class="jt-tok jt-${seg.cls}">${escapeHtml(seg.text)}</span>`
}

function renderHighlighted(
  line: Line,
  lineMatches: Range[] | undefined,
  startGlobal: number,
): string {
  if (!lineMatches?.length) return line.segments.map(tokHtml).join('')
  let out = ''
  let offset = 0
  for (const seg of line.segments) {
    const start = offset
    const end = offset + seg.text.length
    let sliceFrom = 0
    out += `<span class="jt-tok jt-${seg.cls}">`
    lineMatches.forEach(([ms, me], i) => {
      if (me <= start || ms >= end) return
      const s = Math.max(ms, start)
      const e = Math.min(me, end)
      out += escapeHtml(seg.text.slice(sliceFrom, s - start))
      out += `<mark class="jt-mark" data-mi="${startGlobal + i}">${escapeHtml(
        seg.text.slice(s - start, e - start),
      )}</mark>`
      sliceFrom = e - start
    })
    out += escapeHtml(seg.text.slice(sliceFrom))
    out += '</span>'
    offset = end
  }
  return out
}

/** 激活匹配滚动到可视区。
 * 使用 `flush: 'post'` 同步执行而非 `await nextTick()` 的异步 watcher：
 * 异步 continuation 可能在组件卸载后才运行，触碰已销毁实例会引发
 * Vue 内部（`shouldUpdateComponent` 读到空 `emitsOptions`）崩溃。 */
let disposed = false
onBeforeUnmount(() => {
  disposed = true
})

/** 激活匹配的 DOM 级切换：避免上/下导航触发 HTML 全量重建。 */
function applyActiveMark(): void {
  if (disposed) return
  const root = rootRef.value
  if (!root) return
  root.querySelector('.jt-mark.active')?.classList.remove('active')
  const el = root.querySelector(`.jt-mark[data-mi="${props.activeMatch}"]`)
  el?.classList.add('active')
  el?.scrollIntoView({ block: 'nearest' })
}

watch(() => props.activeMatch, applyActiveMark, { flush: 'post' })
// 查找词/数据变化导致 HTML 重建后，重新挂上激活态。
watch(lineHtmls, () => applyActiveMark(), { flush: 'post' })
// 首挂即应用（无 query 时 querySelector 为空操作，开销可忽略）。
onMounted(() => applyActiveMark())

// ---------- 展开 / 收起全部（O(1) 标记，见 forceOpen 说明） ----------
function expandAll(): void {
  forceOpen.value = true
}

function collapseAll(): void {
  forceOpen.value = false
}

defineExpose({ expandAll, collapseAll, matchCount })
</script>

<template>
  <div ref="rootRef" class="jt">
    <div v-for="(line, i) in lines" :key="i" class="jt-line">
      <span class="jt-gutter">{{ i + 1 }}</span>
      <!-- 折叠列：箭头专列（所有行保留等宽空位），不挤占代码位 -->
      <span class="jt-fold">
        <button
          v-if="line.toggleable"
          type="button"
          class="jt-toggle"
          :class="{ open: line.open }"
          :aria-label="line.open ? t('jsonTree.collapse') : t('jsonTree.expand')"
          @click="toggle(line.toggleable, line.open ?? false)"
        >
          <Icon :name="line.open ? 'chevron-down' : 'chevron-right'" :size="12" />
        </button>
      </span>
      <span class="jt-indent" :style="{ width: `${line.depth * 16}px` }" aria-hidden="true"></span>
      <span class="jt-code" v-tooltip-overflow="line.title ?? ''" v-html="lineHtmls[i]"></span>
    </div>
  </div>
</template>

<style scoped>
.jt {
  font-family: var(--font-mono);
  font-size: 12.5px;
  line-height: 1.55;
}

.jt-line {
  display: flex;
  align-items: center;
  min-width: 0;
  white-space: pre;
  color: var(--text-1);
}

.jt-gutter {
  flex-shrink: 0;
  /* 与请求编辑器共用 --code-gutter-* 几何（style.css 单源），代码左缘绝对对齐 */
  width: var(--code-gutter-w);
  padding-right: var(--code-gutter-gap);
  text-align: right;
  /* user-select:none 只挡住「从行号起选」这一种路径；从正文往左拖、或
   * 三击/全选时，浏览器仍会把跨过的行号一起涂蓝（正文与行号同处一个
   * 选择流，不同于 CodeMirror 把 gutter 放在独立 DOM 层）。
   * 故再抹掉行号自身的选区着色，使复制观感与请求编辑器一致。 */
  user-select: none;
  color: var(--tok-gutter);
  font-size: var(--fs-xxs);
}
.jt-gutter::selection {
  /* 行号被扫过时不显示高亮：选区视觉上只在正文上 */
  background-color: transparent;
  color: var(--tok-gutter);
}

/* 折叠列：箭头专列，所有行等宽保留（空行占位），代码左缘不随有无箭头跳动 */
.jt-fold {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: var(--code-fold-w);
}

/* 层级缩进只推移代码（含折叠箭头），行号槽保持固定——编辑器式行号栏 */
.jt-indent {
  flex-shrink: 0;
}

.jt-toggle {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  margin: 0;
  padding: 0;
  border: none;
  border-radius: 4px;
  background: none;
  color: var(--text-2);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.jt-toggle:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.jt-toggle.open {
  color: var(--accent);
}

.jt-code {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 语法着色（--tok-* 统一色阶，与请求 Body 编辑器共用，见 constants/editorTheme.ts）。
 * 行内 token 由 v-html 动态注入，无 scoped 属性，需用 :deep 穿透。 */
:deep(.jt-tok.jt-key) {
  color: var(--tok-key);
}
:deep(.jt-tok.jt-str) {
  color: var(--tok-str);
}
:deep(.jt-tok.jt-num) {
  color: var(--tok-num);
}
:deep(.jt-tok.jt-bool) {
  color: var(--tok-bool);
}
:deep(.jt-tok.jt-null) {
  color: var(--tok-null);
  font-style: italic;
}
:deep(.jt-tok.jt-punct) {
  color: var(--tok-punct);
}
:deep(.jt-tok.jt-dots) {
  color: var(--tok-gutter);
  font-style: italic;
}
:deep(.jt-tok.jt-meta) {
  color: var(--tok-gutter);
  font-style: italic;
  font-size: var(--fs-xxs);
}

/* 查找高亮：普通匹配低对比，当前匹配高亮并描边。 */
:deep(.jt-mark) {
  background: var(--accent-tint, rgba(99, 102, 241, 0.25));
  color: inherit;
  border-radius: 2px;
  padding: 0 1px;
}
:deep(.jt-mark.active) {
  background: var(--accent);
  color: #fff;
  outline: 1px solid var(--accent);
  outline-offset: 1px;
}
</style>