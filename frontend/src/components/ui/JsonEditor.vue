<script setup lang="ts">
/**
 * JsonEditor：JSON 编辑区（暗色代码编辑器风格）。
 * - 覆盖层方案：透明 textarea 叠加在高亮 <pre> 上（零依赖，滚动同步）；
 * - 左侧行号栏：随内容垂直平移、细边框与正文分隔；
 * - 顶部工具栏（非悬浮）：左侧校验状态 Tag，右侧 美化 / 压缩 / 查找 / 复制 按钮；
 *   编辑区内没有任何绝对定位浮层遮挡代码。
 * - 深色底色 #121318，聚焦时 1px 紫色光晕（原 3px 重描边移除）。
 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useToast } from '../../composables/useToast'
import { useLocaleStore } from '../../stores/locale'
import { copyText } from '../../utils/clipboard'
import { highlightJSON, highlightJSONText } from '../../utils/highlight'
import { handleTextareaTab } from '../../utils/textareaIndent'
import { compactJson, prettyJson } from '../../utils/jsonFormat'
import { JsonFormatError } from '../../utils/jsonFormat'
import Icon from './Icon.vue'

const props = withDefaults(
  defineProps<{
    modelValue: string
    placeholder?: string
    /** 编辑区最小高度（px）。 */
    minHeight?: number
    /** 查找词（实时/防抖搜索匹配并高亮）。 */
    query?: string
    /** 当前选中的匹配索引（0-based）。 */
    activeMatch?: number
    /** 自适应内容高度（贴内容生长，min~max 夹取）；false（默认）= 填满外层分派的高度。 */
    autoHeight?: boolean
    /** autoHeight 模式的高度上限（px），超出转编辑器内部滚动。 */
    maxHeight?: number
    /** 工具栏内渲染校验状态 Tag；false 时不渲染（宿主自行经 status-change 事件在外部展示）。 */
    showStatus?: boolean
  }>(),
  {
    placeholder: '',
    minHeight: 120,
    query: '',
    activeMatch: 0,
    autoHeight: false,
    maxHeight: 480,
    showStatus: true,
  },
)

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void
  (e: 'match-count', total: number): void
  (e: 'status-change', status: 'empty' | 'ok' | 'invalid' | 'large'): void
}>()

const toast = useToast()
const locale = useLocaleStore()
const t = locale.t
const taRef = ref<HTMLTextAreaElement | null>(null)
const preRef = ref<HTMLElement | null>(null)
const wrapRef = ref<HTMLElement | null>(null)
const editorRef = ref<HTMLElement | null>(null)
const scrollTop = ref(0)

/**
 * 大内容保护：超过阈值后停用逐键语法高亮 / JSON 校验 / 行号渲染。
 * 这些计算随内容大小线性放大（每次键入全文 parse + 正则高亮 + v-html 重建
 * + 每行一个行号 div），大文档下会造成明显卡顿与内存高水位，故退化为
 * 纯文本编辑（textarea 直接着色，无覆盖层）。
 */
const LARGE_DOC_CHARS = 200_000

const isLargeDoc = computed(() => props.modelValue.length > LARGE_DOC_CHARS)

/**
 * 高亮/校验用文本：小文档（<20k）即时跟手；中等以上防抖 150ms。
 * 原来每键全量 `highlightJSON + split + JSON.parse`（高亮/行号/校验各一次），
 * 百 KB 文档下每次键入三遍全量扫描；输入本身走 textarea 非受控即时响应，
 * 防抖只延迟着色与状态 Tag，不影响键入手感。
 */
const SHOWN_DEBOUNCE_CHARS = 20_000
const shownText = ref(props.modelValue)
let shownTimer: ReturnType<typeof setTimeout> | undefined
watch(
  () => props.modelValue,
  (v) => {
    if (shownTimer) clearTimeout(shownTimer)
    if (v.length < SHOWN_DEBOUNCE_CHARS) {
      shownText.value = v
      return
    }
    shownTimer = setTimeout(() => {
      shownText.value = v
    }, 150)
  },
)
onUnmounted(() => {
  if (shownTimer) clearTimeout(shownTimer)
  wrapRef.value?.removeEventListener('scroll', clampWrapScroll)
  availObserver?.disconnect()
  availObserver = null
})

/** 空内容渲染一个空格，保证 pre 与 textarea 高度一致（滚动同步前提）。 */
const matchCount = computed(() => {
  if (!props.query || isLargeDoc.value) return 0
  const ql = props.query.toLowerCase()
  const lower = props.modelValue.toLowerCase()
  let n = 0
  let from = 0
  for (;;) {
    const idx = lower.indexOf(ql, from)
    if (idx === -1) break
    n += 1
    from = idx + ql.length
  }
  return n
})

watch(
  matchCount,
  (cnt) => {
    emit('match-count', cnt)
  },
  { immediate: true },
)

/** 空内容渲染一个空格，保证 pre 与 textarea 高度一致（滚动同步前提）。 */
const html = computed(() => {
  if (isLargeDoc.value) return ''
  const val = props.modelValue.length ? props.modelValue : ' '
  if (props.query) {
    return highlightJSONText(val, props.query, props.activeMatch)
  }
  return highlightJSON(val)
})

function getMatchOffsets(text: string, q: string, targetIdx: number): [number, number] | null {
  if (!q) return null
  const ql = q.toLowerCase()
  const lower = text.toLowerCase()
  let from = 0
  let cur = 0
  for (;;) {
    const idx = lower.indexOf(ql, from)
    if (idx === -1) return null
    if (cur === targetIdx) {
      return [idx, idx + q.length]
    }
    cur += 1
    from = idx + ql.length
  }
}

watch(
  () => [props.query, props.activeMatch],
  ([q, matchIdx]) => {
    if (!q || !taRef.value || matchIdx === undefined) return
    const offsets = getMatchOffsets(props.modelValue, String(q), Number(matchIdx))
    if (!offsets) return
    const [start, end] = offsets
    taRef.value.setSelectionRange(start, end)
    const linesBefore = props.modelValue.slice(0, start).split('\n').length - 1
    // 正文行盒 = --fs-sm × 1.55（与 .hl-pre/.hl-ta 行高同源，见样式区）
    const lineHeight = 13 * 1.55
    const targetScroll = Math.max(0, linesBefore * lineHeight - 40)
    taRef.value.scrollTop = targetScroll
    if (preRef.value) {
      preRef.value.scrollTop = targetScroll
    }
  },

)

const lineCount = computed(() => (isLargeDoc.value ? 0 : shownText.value.split('\n').length))

/* 自适应高度：行盒 = --fs-sm(13px) × line-height 1.55，另加 wrap 上下 padding 20 与边框 2。
 * 由行数推算而非回读 scrollHeight（ta height:100% 会把旧高度读回来卡住），增删行双向即时跟随。
 * 上限 = 面板可用高度（availH，由请求/响应分割条拖动、窗口缩放经 ResizeObserver 重测）：
 * 内容高过可用高度就填满面板并转框内滚动——否则内容被面板 overflow:hidden 裁掉且
 * 任何地方都没有滚动条、滚轮无反应；内容低于可用高度则贴内容生长（minHeight 兜底）。
 * avail 尚未测得（首帧）时退回 maxHeight，防首帧无界。 */
/** 面板可用高度（.json-editor 盒高 − 工具栏 − 间距，px）；0 = 尚未测得（暂以 maxHeight 夹取）。 */
const availH = ref(0)
const wrapHeight = computed<number | null>(() => {
  if (!props.autoHeight) return null
  if (isLargeDoc.value) return availH.value > 0 ? availH.value : props.maxHeight
  const content = Math.ceil(lineCount.value * 13 * 1.55) + 22
  const h = Math.max(content, props.minHeight)
  return availH.value > 0 ? Math.min(h, availH.value) : Math.min(h, props.maxHeight)
})

const wrapStyle = computed(() => ({
  /* CSS min-height 优先级高于 height：面板被拖到 availH < 地板值时，height 会被
   * min-height 顶回去 → wrap 高出面板可用区、panel overflow:hidden 裁出无滚动条
   * 死区（实测 avail152 被 160 地板顶成 160，底裁 8px）。地板必须钳到 availH。 */
  minHeight: `${availH.value > 0 ? Math.min(props.minHeight, availH.value) : props.minHeight}px`,
  ...(wrapHeight.value ? { height: `${wrapHeight.value}px` } : {}),
}))

/* 行号槽宽度走全局 --code-gutter-w（固定 54px，与响应树/行视图共用，
 * 不随位数伸缩——保证与响应区代码左缘绝对对齐），见 style.css。 */

const status = computed<'empty' | 'ok' | 'invalid' | 'large'>(() => {
  if (isLargeDoc.value) return 'large'
  const t = shownText.value.trim()
  if (!t) return 'empty'
  try {
    JSON.parse(t)
    return 'ok'
  } catch {
    return 'invalid'
  }
})

/** 工具栏状态 Tag 文案与图标（empty 时不渲染）。 */
const statusText = computed(
  () =>
    ({ ok: t('jsonEditor.ok'), invalid: t('jsonEditor.invalid'), large: t('jsonEditor.large') })[
      status.value as 'ok' | 'invalid' | 'large'
    ] ?? '',
)
const statusIcon = computed(() => (status.value === 'ok' ? 'check' : 'x'))

/** 状态外报：宿主可将校验标签展示在编辑器之外（如 Body 模式栏右侧）。 */
watch(
  status,
  (s) => {
    emit('status-change', s)
  },
  { immediate: true },
)

/** 是否有内容可格式化 / 复制（textarea 实时值为权威，见 format 注释）。 */
const hasContent = computed(() => (taRef.value?.value ?? props.modelValue).trim().length > 0)

function onInput(e: Event): void {
  emit('update:modelValue', (e.target as HTMLTextAreaElement).value)
}

/** Tab / Shift+Tab 缩进而非跳出焦点（代码编辑器习惯）。 */
function onTextareaKeydown(e: KeyboardEvent): void {
  if (e.key === 'Tab' && taRef.value) handleTextareaTab(taRef.value, e)
}

function syncScroll(e: Event): void {
  const ta = e.target as HTMLTextAreaElement
  scrollTop.value = ta.scrollTop
  if (preRef.value) {
    preRef.value.scrollTop = ta.scrollTop
    preRef.value.scrollLeft = ta.scrollLeft
  }
}

/**
 * 面板可用高度（wrap 高度上限），两条路径：
 * - 填充/分割模式（autoHeight=false）：.json-editor 盒高由 flex 分派（与 wrap
 *   内容无关，无回环），减去自身工具栏与 4px gap 即 wrap 可用最大高度。
 * - 自适应模式（autoHeight=true，无外层高度分派时）：从 host 向上找**最内层** overflow
 *   被约束的祖先（BodyPanel .panel overflow:hidden，即真正裁剪 host 的盒子），
 *   量它的视口：avail = sc.clientH − (host.top − sc.top) − 工具栏 − 4。
 *   该祖先必须是有界盒（config-box 内 flex:1，高度由外层分派、与 wrap 内容无关）
 *   → wrap 恰好填满它就是不动点；panel 盒高变化必由外层（config-box / 分割条 /
 *   窗口）驱动 → .json-editor 的 ResizeObserver 必然跟进重测，链路自洽。
 *   禁止改回「向上找活动滚动器（sh>c）再量」：外层（如 .config-box）溢出时会量到
 *   外层视口——没算 host 下方 footer/占位条的高度，高估几十 px；而 wrap 因此收缩
 *   并不改 .json-editor 盒高（flex:1 自顶向下），RO 不再触发，avail 永久卡在瞬态
 *   高值 → 面板底部裁出无滚动条死区（headless 实测卡 263，真值 214）。
 *   找不到有界祖先（页面级滚动）不设上限，交给 maxHeight。
 */
function measureAvail(): void {
  const host = editorRef.value
  if (!host) return
  const toolbar = host.querySelector<HTMLElement>(':scope > .je-toolbar')
  const tbar = toolbar?.offsetHeight ?? 0
  if (props.autoHeight) {
    let sc: HTMLElement | null = null
    let cur: HTMLElement | null = host.parentElement
    while (cur) {
      const oy = getComputedStyle(cur).overflowY
      if (oy === 'auto' || oy === 'scroll' || oy === 'hidden' || oy === 'clip') {
        sc = cur
        break
      }
      cur = cur.parentElement
    }
    if (!sc) {
      availH.value = 0
      return
    }
    const topInView = host.getBoundingClientRect().top - sc.getBoundingClientRect().top
    availH.value = Math.max(0, Math.round(sc.clientHeight - topInView - tbar - 4))
    return
  }
  availH.value = Math.max(0, host.clientHeight - tbar - 4)
}

/**
 * 面板可用高度重测（availObserver）：请求卡高度变化（分割条拖动 / 窗口缩放 /
 * 模式切换）时下一帧重测 availH，wrap 上限即时跟随——无「拖拽 → 观测 → 回写」
 * 环，也就没有 ResizeObserver loop 告警。
 */
let availObserver: ResizeObserver | null = null
let roFramePending = false
/**
 * ResizeObserver 回调统一挪到下一帧再测：回调里写响应式状态会让 Vue 在同一
 * 渲染机会内改 DOM，内核补投递新一轮观察时报「ResizeObserver loop completed
 * with undelivered notifications」（WKWebView 会把它当窗口错误弹到
 * ErrorBoundary）。rAF 在样式/布局前收尾本轮，新观察下一帧干净投递。
 */
function nextFrame(cb: () => void): void {
  if (typeof requestAnimationFrame === 'function') requestAnimationFrame(cb)
  else setTimeout(cb, 16)
}
function scheduleRoSync(): void {
  if (roFramePending) return
  roFramePending = true
  let done = false
  const run = (): void => {
    if (done) return
    done = true
    roFramePending = false
    measureAvail()
  }
  nextFrame(run)
  // rAF 被节流的环境（后台/隐藏 webview、headless）里 rAF 回调可能长期不投递，
  // roFramePending 会永久卡住 → avail 从此不再更新（实测过挂载后一次都没再测）。
  // setTimeout 不会被彻底停摆，超时强制补测一帧；rAF 正常时 done 守卫直接短路。
  setTimeout(run, 300)
}

/** 老内核兜底：wrap 被链式滚动后整体（行号槽 + 高亮层 + textarea）左移/上移错乱。 */
function clampWrapScroll(): void {
  const wrap = wrapRef.value
  if (!wrap) return
  wrap.scrollLeft = 0
  wrap.scrollTop = 0
}

onMounted(() => {
  measureAvail()
  // 兜底：老内核不支持 overflow: clip 时 wrap 仍是滚动容器，横向滚轮会链式
  // 滚到 wrap 上（行号槽 + 正文整体左移错乱），强制归零。
  wrapRef.value?.addEventListener('scroll', clampWrapScroll, { passive: true })
  if (typeof ResizeObserver === 'undefined') return
  // 面板可用高度：容器盒高变化（窗口缩放 / 分割条拖动 / 模式切换）时重测。
  if (editorRef.value) {
    availObserver = new ResizeObserver(scheduleRoSync)
    availObserver.observe(editorRef.value)
    // 工具栏自身高度（字体 swap、状态文案增减）会吃掉 avail，但不改
    // .json-editor 盒高 → 主观察者看不见，必须单独观察，否则 avail 永久偏大、
    // 面板底裁出无滚动条死区。
    const toolbar = editorRef.value.querySelector<HTMLElement>(':scope > .je-toolbar')
    if (toolbar) availObserver.observe(toolbar)
  }
  // 字体加载晚于首帧测量时（WebFont swap）工具栏高度会变，上面的观察者会跟进；
  // 这里再补一枪覆盖「字体就绪但盒子没动」的窄缝。
  try {
    document.fonts?.ready.then(() => scheduleRoSync())
  } catch {
    /* 老内核无 FontFaceSet，忽略 */
  }
})

// 内容变化（格式化 / 粘贴 / 回退 / 外部保存）后重新对齐滚动与 DOM 同步，避免高亮层与行号错位。
watch(
  () => props.modelValue,
  (newVal) => {
    if (taRef.value && taRef.value.value !== newVal) {
      taRef.value.value = newVal
    }
    if (taRef.value && preRef.value) {
      preRef.value.scrollTop = taRef.value.scrollTop
      preRef.value.scrollLeft = taRef.value.scrollLeft
      scrollTop.value = taRef.value.scrollTop
    }
  },
)

function format(mode: 'pretty' | 'compact'): void {
  // 以 textarea 的实时 DOM 值为权威来源：真实浏览器里 input 事件可能丢失或
  // 延迟（拖拽写入、自动填充、受控回写竞态），导致 props.modelValue 落后于
  // 用户实际编辑的内容——此时若按 props 格式化会把编辑器回退成旧数据。
  const text = (taRef.value?.value ?? props.modelValue).trim()
  if (!text) return
  try {
    // 无损格式化：保留重复键、键顺序与数字原文（parse/stringify 往返会丢重复键）。
    const out = mode === 'compact' ? compactJson(text) : prettyJson(text)
    if (out !== props.modelValue) {
      emit('update:modelValue', out)
    } else if (taRef.value && taRef.value.value !== out) {
      // 模型已一致但 DOM 残留未同步的值：直接纠正 DOM。
      taRef.value.value = out
    }
    toast.success(mode === 'compact' ? t('jsonEditor.compacted') : t('jsonEditor.prettified'))
    void nextTick(() => {
      // 双保险：emit 回写后强制对齐 DOM（WebKit 偶发不回写受控 value）。
      if (taRef.value && taRef.value.value !== props.modelValue) {
        taRef.value.value = props.modelValue
      }
      taRef.value?.focus()
    })
  } catch (err) {
    toast.error(err instanceof JsonFormatError ? t('jsonEditor.invalidJson', { v: err.message }) : t('jsonEditor.invalidJsonPlain'))
  }
}

async function copyJson(): Promise<void> {
  const text = taRef.value?.value ?? props.modelValue
  if (!text.trim()) return
  const ok = await copyText(text)
  if (ok) toast.success(t('jsonEditor.copied'))
  else toast.error(t('response.copyFail'))
}
</script>

<template>
  <div ref="editorRef" class="json-editor">
    <!-- 顶部工具栏：状态 Tag + 快捷操作（替代原悬浮层，不遮挡代码） -->
    <div class="je-toolbar">
      <span
        v-if="showStatus && status !== 'empty'"
        class="je-status"
        :class="status"
        :title="
          status === 'invalid'
            ? t('jsonEditor.invalidHint')
            : status === 'large'
              ? t('jsonEditor.largeHint')
              : t('jsonEditor.okHint')
        "
      >
        <Icon v-if="status === 'ok' || status === 'invalid'" :name="statusIcon" :size="10" />
        <span v-else class="je-dot" aria-hidden="true"></span>
        {{ statusText }}
      </span>
      <span v-else class="je-toolbar-spacer" aria-hidden="true"></span>
      <div class="je-actions">
        <button
          class="je-btn"
          type="button"
          :title="t('jsonEditor.prettyHint')"
          :aria-label="t('jsonEditor.prettyHint')"
          :disabled="!hasContent"
          @click="format('pretty')"
        >
          <Icon name="zap" :size="12" />
        </button>
        <button
          class="je-btn"
          type="button"
          :title="t('jsonEditor.compactHint')"
          :aria-label="t('jsonEditor.compactHint')"
          :disabled="!hasContent"
          @click="format('compact')"
        >
          <Icon name="minimize-2" :size="12" />
        </button>
        <button
          class="je-btn"
          type="button"
          :title="t('jsonEditor.copyHint')"
          :aria-label="t('jsonEditor.copyHint')"
          :disabled="!hasContent"
          @click="copyJson"
        >
          <Icon name="copy" :size="12" />
        </button>
      </div>
    </div>

    <div ref="wrapRef" class="hl-wrap" :class="{ 'auto-height': autoHeight }" :style="wrapStyle">
      <div v-if="!isLargeDoc" class="hl-gutter" aria-hidden="true">
        <div
          class="hl-gutter-inner"
          :style="{ transform: `translateY(${-scrollTop}px)` }"
        >
          <div v-for="n in lineCount" :key="n" class="hl-gutter-line">{{ n }}</div>
        </div>
      </div>
      <pre
        v-if="!isLargeDoc"
        ref="preRef"
        class="hl-pre"
        aria-hidden="true"
        v-html="html"
      ></pre>
      <textarea
        ref="taRef"
        class="hl-ta"
        :class="{ plain: isLargeDoc }"
        :value="modelValue"
        :placeholder="placeholder"
        spellcheck="false"
        @input="onInput"
        @change="onInput"
        @scroll="syncScroll"
        @keydown="onTextareaKeydown"
      ></textarea>
    </div>
  </div>
</template>

<style scoped>
.json-editor {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
  min-height: 0;
}

.hl-wrap {
  position: relative;
  border: 1px solid var(--border-editor);
  border-radius: var(--radius);
  background: var(--bg-code);
  /* clip：wrap 不再是滚动容器——overflow:hidden 仍可被滚轮链式滚动，横向长
   * 内容滑到底后继续滑会把行号槽 + 正文整体左移错乱；老内核不识别 clip 时
   * 退回 hidden，另有 JS 归零兜底（clampWrapScroll）。 */
  overflow: hidden;
  overflow: clip;
  flex: 1;
  min-height: 0;
  transition:
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}

/* 自适应高度模式：贴内容生长（上限 availH），不参与 flex 抢高。
 * 高度调整入口在请求面板物理底边的 rp-splitter（EndpointEditor），编辑器只跟随。 */
.hl-wrap.auto-height {
  flex: 0 0 auto;
}

/* 1px 紫色聚焦光晕（替代原 3px 重描边） */
.hl-wrap:focus-within {
  border-color: rgba(168, 85, 247, 0.6);
  box-shadow: 0 0 0 1px rgba(168, 85, 247, 0.5);
}

.hl-pre,
.hl-ta {
  margin: 0;
  padding: 10px 12px;
  /* 正文让出「行号槽 + 折叠列」整条前区（与响应树同值同源，代码左缘绝对对齐） */
  padding-left: calc(var(--code-gutter-w) + var(--code-fold-w));
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  line-height: 1.55;
  white-space: pre;
  tab-size: 2;
}

.hl-pre {
  position: absolute;
  inset: 0;
  color: var(--code-fg);
  pointer-events: none;
  overflow: hidden;
  word-break: normal;
}

.hl-ta {
  position: relative;
  width: 100%;
  height: 100%;
  display: block;
  box-sizing: border-box;
  /* resize 关闭：高度归 wrap 单向驱动（wrap 又由面板可用高度 availH 驱动，
   * 即请求/响应分割条拖动），原生角标各内核表现不一（WebKit 不绘制） */
  resize: none;
  border: none;
  outline: none;
  background: transparent;
  color: transparent;
  caret-color: var(--code-caret);
  overflow: auto;
}
.hl-ta::placeholder {
  color: var(--text-3);
}

/* 纯文本模式：大文档降级，直接在 textarea 上着色（无行号槽，回到常规左距） */
.hl-ta.plain {
  color: var(--text-1);
  background: var(--code-bg);
  padding-left: 12px;
}

/* ---- 语法高亮色系 ---- */
:deep(.hl-k) {
  color: var(--tok-key, #e06c75);
}
:deep(.hl-s) {
  color: var(--tok-str, #98c379);
}
:deep(.hl-n) {
  color: var(--tok-num, #d19a66);
}
:deep(.hl-b) {
  color: var(--tok-bool, #56b6c2);
}
:deep(.hl-null) {
  color: var(--tok-null, #56b6c2);
  font-style: italic;
}
:deep(.hl-p) {
  color: var(--tok-punct, #abb2bf);
}
:deep(.hl-c) {
  color: var(--tok-gutter, #5c6370);
  font-style: italic;
}

/* ---- 查找标记 ---- */
:deep(.rp-find-mark) {
  background: var(--accent-tint, rgba(99, 102, 241, 0.25));
  color: inherit;
  border-radius: 2px;
  padding: 0 1px;
}
:deep(.rp-find-mark.active) {
  background: var(--accent, #a855f7);
  color: #fff;
  outline: 1px solid var(--accent, #a855f7);
  outline-offset: 1px;
}

/* 行号栏：与响应区共用 --code-gutter-* 几何（宽度/间隙单源），
 * 底色取 --bg-code——与代码底色同色，静止时视觉仍是「数字落在代码底色上」，
 * 但必须不透明：pre/ta 满宽铺底，横向滚动时正文会滑到行号槽底下，
 * 透明则数字与正文交叠错乱 */
.hl-gutter {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: var(--code-gutter-w);
  background: var(--bg-code);
  overflow: hidden;
  user-select: none;
  pointer-events: none;
  z-index: 1;
}
.hl-gutter-inner {
  padding: 10px 0;
  will-change: transform;
}
.hl-gutter-line {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  /* 行号行盒必须与正文行盒同高（--fs-sm 13px × 1.55 = 20.15px）：字面可比正文
   * 小一档，但行高由正文推导——否则行号槽逐行上漂（17.05 vs 20.15），滚动时
   * 行号与内容错位、行号先于内容走完 */
  height: calc(var(--fs-sm) * 1.55);
  line-height: calc(var(--fs-sm) * 1.55);
  color: var(--tok-gutter, #5c6370);
  text-align: right;
  padding-right: var(--code-gutter-gap);
}

/* 工具栏 */
.je-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 2px 2px 4px;
}
.je-toolbar-spacer {
  flex: 1;
}

/* 校验状态 Tag：弱化的次级信息（10px 微字 + 紧凑内距），不与代码争夺注意力 */
.je-status {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 6px;
  border-radius: 9999px;
  font-size: var(--fs-xxs);
  font-weight: 500;
  line-height: 1.4;
}
.je-status.ok {
  background: var(--success-tint);
  color: var(--success);
}
.je-status.invalid {
  background: var(--danger-tint);
  color: var(--danger);
}
.je-status.large {
  background: var(--warning-tint);
  color: var(--warning);
}

.je-dot {
  width: 4px;
  height: 4px;
  border-radius: 9999px;
  background: currentColor;
}

.je-actions {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  margin-left: auto;
}

.je-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.je-btn:hover:not(:disabled) {
  background: var(--bg-hover);
  border-color: var(--border);
  color: var(--text-1);
}
.je-btn.active {
  background: var(--accent-tint, rgba(168, 85, 247, 0.15));
  color: var(--accent, #a855f7);
  border-color: var(--accent, #a855f7);
}
.je-btn:disabled {
  opacity: 0.35;
  cursor: default;
}
</style>
