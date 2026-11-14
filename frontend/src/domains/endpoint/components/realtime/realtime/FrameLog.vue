<script setup lang="ts">
/**
 * FrameLog：实时调试帧日志列表（WS / SSE 共用）。
 *
 * - 统一帧形状 FrameItem（宿主把 WS message / SSE chunk 都映射进来）；
 * - 过滤：关键字（内容/类型/事件名）+ 方向 chips（全部/收/发/系统）；
 * - 虚拟滚动：定高 28px 行 + useVirtualList（<80 行回退全量渲染）；
 * - 自动滚底：新帧贴底；用户上滚（离底 > 40px）即暂停，工具按钮恢复并跳底；
 * - 行点击 → emit('select')，选中行高亮由父级传 selectedSeq 回填；
 * - a11y：容器 role=log + aria-live=polite（辅助技术可感知新帧）。
 */
import { computed, ref, watch } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { useVirtualList } from '@/composables/useVirtualList'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import EmptyState from '@/components/ui/EmptyState.vue'

/** 统一帧形状：WS message / SSE chunk / 系统事件映射后的最小渲染单元。 */
export interface FrameItem {
  /** 单调递增 id（cap 裁剪后 key 仍稳定）。 */
  seq: number
  /** HH:mm:ss。 */
  t: string
  dir: 'in' | 'out' | 'sys'
  /** text | binary | ping | sse | sys。 */
  kind: string
  text: string
  /** 字符数（binary 为 base64 文本长度）。 */
  size: number
  /** SSE 帧的事件名（event: 行）与 id:（续传位点）；其余帧无。 */
  meta?: { id?: string; event?: string }
}

const props = defineProps<{
  items: FrameItem[]
  /** 父级选中的帧 seq（高亮回显）。 */
  selectedSeq?: number | null
  /** 无任何帧时的空态文案（标题）。 */
  emptyTitle: string
}>()

const emit = defineEmits<{
  select: [item: FrameItem]
}>()

const locale = useLocaleStore()
const t = locale.t

// ---------- 过滤 ----------
const query = ref('')
const dirFilter = ref<'all' | 'in' | 'out' | 'sys'>('all')

const filtered = computed(() => {
  const q = query.value.trim().toLowerCase()
  return props.items.filter((m) => {
    if (dirFilter.value !== 'all' && m.dir !== dirFilter.value) return false
    if (!q) return true
    return (
      m.text.toLowerCase().includes(q) ||
      m.kind.toLowerCase().includes(q) ||
      (m.meta?.event ?? '').toLowerCase().includes(q) ||
      (m.meta?.id ?? '').toLowerCase().includes(q)
    )
  })
})

const DIRS: { key: 'all' | 'in' | 'out' | 'sys'; label: string }[] = [
  { key: 'all', label: t('realtime.dirAll') },
  { key: 'in', label: t('realtime.dirIn') },
  { key: 'out', label: t('realtime.dirOut') },
  { key: 'sys', label: t('realtime.dirSys') },
]

// ---------- 虚拟滚动 + 自动滚底 ----------
const listEl = ref<HTMLElement | null>(null)
const ROW_H = 28
const { virtualItems, offsetY, totalHeight, onScroll, enabled } = useVirtualList(
  listEl,
  filtered,
  { itemHeight: ROW_H, overscan: 8, threshold: 80 },
)

/** 自动滚底：新帧贴底；用户上滚离底 > 40px 即暂停。 */
const follow = ref(true)
/** 是否因用户滚动暂停（用于按钮态区分「本来就关」）。 */
function isNearBottom(): boolean {
  const el = listEl.value
  if (!el) return true
  return el.scrollHeight - el.scrollTop - el.clientHeight < 40
}
function scrollToBottom(): void {
  const el = listEl.value
  if (el) el.scrollTop = el.scrollHeight
}
watch(
  () => filtered.value.length,
  () => {
    if (follow.value && enabled.value) requestAnimationFrame(scrollToBottom)
  },
)
function onListScroll(): void {
  onScroll()
  if (enabled.value && !isNearBottom()) follow.value = false
}
function resumeFollow(): void {
  follow.value = true
  scrollToBottom()
}

// 过滤条件变化后重置跟随（用户主动动过滤 = 想看最新匹配）
watch([query, dirFilter], () => {
  follow.value = true
  requestAnimationFrame(scrollToBottom)
})

function copyFrame(item: FrameItem): void {
  void navigator.clipboard.writeText(item.text).catch(() => {})
}

function dirGlyph(dir: FrameItem['dir']): string {
  return dir === 'in' ? '↓' : dir === 'out' ? '↑' : '•'
}
</script>

<template>
  <div class="fl">
    <!-- 过滤条 -->
    <div class="fl-bar">
      <div class="fl-search">
        <Icon name="search" :size="12" class="fl-search-icon" />
        <input
          v-model="query"
          class="fl-search-input"
          type="text"
          :placeholder="t('realtime.filterPh')"
          spellcheck="false"
        />
      </div>
      <div class="fl-dirs" role="group" :aria-label="t('realtime.dirFilterGroup')">
        <button
          v-for="d in DIRS"
          :key="d.key"
          type="button"
          class="fl-dir-chip"
          :class="{ on: dirFilter === d.key }"
          @click="dirFilter = d.key"
        >
          {{ d.label }}
        </button>
      </div>
      <span class="fl-count num">{{ filtered.length }}/{{ items.length }}</span>
      <IconButton
        name="chevron-down"
        :size="13"
        :tone="follow ? 'accent' : 'default'"
        :title="follow ? t('realtime.followOn') : t('realtime.followOff')"
        @click="resumeFollow"
      />
    </div>

    <!-- 帧列表（定高行虚拟滚动） -->
    <div
      ref="listEl"
      class="fl-list"
      role="log"
      aria-live="polite"
      @scroll="onListScroll"
    >
      <EmptyState
        v-if="!items.length"
        icon="terminal"
        :title="emptyTitle"
        compact
      />
      <p v-else-if="!filtered.length" class="fl-nomatch">{{ t('realtime.noMatch') }}</p>
      <div v-else :style="enabled ? { height: `${totalHeight}px`, position: 'relative' } : undefined">
        <div
          :style="enabled
            ? { position: 'absolute', left: 0, right: 0, top: 0, transform: `translateY(${offsetY}px)` }
            : undefined"
        >
          <div
            v-for="it in virtualItems"
            :key="it.data.seq"
            class="fl-row"
            :class="[`dir-${it.data.dir}`, { selected: it.data.seq === selectedSeq }]"
            :style="{ height: `${ROW_H}px` }"
            role="button"
            tabindex="0"
            @click="emit('select', it.data)"
            @keydown.enter="emit('select', it.data)"
          >
            <span class="fl-glyph" :title="it.data.dir">{{ dirGlyph(it.data.dir) }}</span>
            <span class="fl-time num">{{ it.data.t }}</span>
            <span class="fl-kind">{{ it.data.meta?.event ?? it.data.kind }}</span>
            <span v-if="it.data.meta?.id" class="fl-id">#{{ it.data.meta.id }}</span>
            <span class="fl-text">{{ it.data.text }}</span>
            <span class="fl-size num" :title="t('realtime.chars', { n: it.data.size })">{{ it.data.size }}</span>
            <IconButton
              class="fl-copy"
              name="copy"
              :size="12"
              :title="t('common.copyAction')"
              @click.stop="copyFrame(it.data)"
            />
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.fl {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex: 1;
  min-height: 0;
  min-width: 0;
}

/* ---- 过滤条 ---- */
.fl-bar {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}
.fl-search {
  position: relative;
  flex: 1;
  min-width: 120px;
}
.fl-search-icon {
  position: absolute;
  left: 8px;
  top: 50%;
  transform: translateY(-50%);
  color: var(--text-3);
  pointer-events: none;
}
.fl-search-input {
  width: 100%;
  height: var(--h-sm);
  padding: 0 8px 0 26px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius);
  background: var(--bg-card);
  color: var(--text-1);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  outline: none;
  transition: border-color var(--dur) var(--ease), box-shadow var(--dur) var(--ease);
}
.fl-search-input:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint);
}
.fl-search-input::placeholder {
  color: var(--text-3);
}

.fl-dirs {
  display: inline-flex;
  gap: 2px;
  padding: 2px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-card);
  flex-shrink: 0;
}
.fl-dir-chip {
  height: 22px;
  padding: 0 8px;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-3);
  font-size: var(--fs-xxs);
  font-family: inherit;
  cursor: pointer;
  white-space: nowrap;
  transition: color var(--dur) var(--ease), background var(--dur) var(--ease);
}
.fl-dir-chip:hover {
  color: var(--text-1);
}
.fl-dir-chip.on {
  background: var(--accent-tint);
  color: var(--accent);
  font-weight: 600;
}

.fl-count {
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
  white-space: nowrap;
}

/* ---- 帧列表 ---- */
.fl-list {
  flex: 1;
  min-height: 160px;
  overflow-y: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-code);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
}
.fl-nomatch {
  margin: 0;
  padding: 16px;
  text-align: center;
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.fl-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 10px;
  cursor: pointer;
  white-space: nowrap;
  border-left: 2px solid transparent;
  transition: background var(--dur) var(--ease);
}
.fl-row:hover {
  background: var(--bg-hover);
}
.fl-row.selected {
  background: var(--accent-tint);
  border-left-color: var(--accent);
}

.fl-glyph {
  flex: none;
  width: 12px;
  text-align: center;
  font-weight: 700;
}
.dir-in .fl-glyph {
  color: var(--success);
}
.dir-out .fl-glyph {
  color: var(--info);
}
.dir-sys .fl-glyph {
  color: var(--text-3);
}

.fl-time {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.fl-kind {
  flex: none;
  padding: 0 5px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-2);
  font-size: var(--fs-2xxs);
  line-height: 16px;
  text-transform: uppercase;
  letter-spacing: 0.03em;
}
.dir-in .fl-kind {
  color: var(--success);
  background: var(--success-tint);
}
.dir-out .fl-kind {
  color: var(--info);
  background: var(--info-tint);
}
.fl-id {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.fl-text {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  color: var(--text-1);
}
.dir-sys .fl-text {
  color: var(--text-2);
  font-style: italic;
}
.fl-size {
  flex: none;
  min-width: 34px;
  text-align: right;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.fl-copy {
  flex: none;
  opacity: 0;
  transition: opacity var(--dur) var(--ease);
}
.fl-row:hover .fl-copy,
.fl-copy:focus-visible {
  opacity: 1;
}
</style>
