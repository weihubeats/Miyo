<script setup lang="ts">
/**
 * CreatePanel：目录工具栏「+」的新建面板（参考 Apifox 式布局）。
 * - 分组标题（新建 / 其他）+ 首组双列大卡片（彩色图标徽章）；
 * - 只收录真实能力（新建请求 / 文件夹 / cURL 导入 / 文档导入），无占位项；
 * - Teleport 到 body（避免被 overflow 裁剪），触发器下方左对齐展开，
 *   下方空间不足自动上翻；外部点击 / Esc / 滚动 / 缩放关闭；
 * - 键盘：↑/↓ 移动高亮（循环）、Enter 选中、Esc 关闭。
 */
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import Icon from '@/components/ui/Icon.vue'
import type { IconName } from '@/components/ui/Icon.vue'

export type CreateActionKey = 'new-request' | 'new-folder' | 'import-curl' | 'import-doc'

interface CreateEntry {
  key: CreateActionKey
  label: string
  icon: IconName
  badge: 'blue' | 'amber' | 'green' | 'purple'
  shortcut?: string
}

const locale = useLocaleStore()
const t = locale.t

const emit = defineEmits<{
  select: [key: CreateActionKey]
  open: []
  close: []
}>()

const open = ref(false)
const highlight = ref(0)
const panelEl = ref<HTMLDivElement | null>(null)
const pos = ref({ left: 0, top: 0 })
/** 打开时的触发元素：外部 mousedown 落在它上面时不关闭（由按钮 click 接管开关）。 */
let anchorEl: HTMLElement | null = null

/** 首组双列卡片 + 次组行（与 WorkspaceView 的真实动作一一对应）。 */
const groups = computed(() => [
  {
    title: t('createpanel.newTitle'),
    layout: 'grid' as const,
    entries: [
      { key: 'new-request', label: t('workspace.newRequest'), icon: 'zap', badge: 'blue', shortcut: '⌘N' },
      { key: 'new-folder', label: t('workspace.newFolder'), icon: 'folder-plus', badge: 'amber' },
    ] as CreateEntry[],
  },
  {
    title: t('createpanel.otherTitle'),
    layout: 'list' as const,
    entries: [
      { key: 'import-curl', label: t('workspace.importCurl'), icon: 'terminal', badge: 'green' },
      { key: 'import-doc', label: t('workspace.importDoc'), icon: 'upload', badge: 'purple' },
    ] as CreateEntry[],
  },
])

const flatEntries = computed(() => groups.value.flatMap((g) => g.entries))

function openAt(anchor: HTMLElement): void {
  anchorEl = anchor
  highlight.value = 0
  open.value = true
  emit('open')
  void nextTick(() => position(anchor))
}

function position(anchor: HTMLElement): void {
  const rect = anchor.getBoundingClientRect()
  const width = 360
  const height = Math.min(420, 200 + flatEntries.value.length * 10)
  const spaceBelow = window.innerHeight - rect.bottom - 8
  const up = spaceBelow < height && rect.top > height
  pos.value = {
    left: Math.max(8, Math.min(rect.left, window.innerWidth - width - 8)),
    top: up ? Math.max(8, rect.top - height - 8) : rect.bottom + 8,
  }
}

function close(): void {
  if (!open.value) return
  open.value = false
  emit('close')
}

function pick(key: CreateActionKey): void {
  close()
  emit('select', key)
}

function onKeydown(event: KeyboardEvent): void {
  if (!open.value) return
  const n = flatEntries.value.length
  if (event.key === 'ArrowDown') {
    event.preventDefault()
    highlight.value = (highlight.value + 1) % n
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    highlight.value = (highlight.value - 1 + n) % n
  } else if (event.key === 'Enter') {
    event.preventDefault()
    const entry = flatEntries.value[highlight.value]
    if (entry) pick(entry.key)
  } else if (event.key === 'Escape') {
    event.preventDefault()
    close()
  }
}

function flatIndexOf(key: CreateActionKey): number {
  return flatEntries.value.findIndex((e) => e.key === key)
}

function onDocMouseDown(event: MouseEvent): void {
  const target = event.target as Node
  if (panelEl.value?.contains(target)) return
  if (anchorEl?.contains(target)) return
  close()
}

function onDocKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') close()
}

function onReposition(): void {
  close()
}

watch(open, (isOpen) => {
  if (isOpen) {
    document.addEventListener('mousedown', onDocMouseDown, true)
    document.addEventListener('keydown', onDocKeydown)
    window.addEventListener('scroll', onReposition, true)
    window.addEventListener('resize', onReposition)
  } else {
    document.removeEventListener('mousedown', onDocMouseDown, true)
    document.removeEventListener('keydown', onDocKeydown)
    window.removeEventListener('scroll', onReposition, true)
    window.removeEventListener('resize', onReposition)
    anchorEl = null
  }
})

onBeforeUnmount(() => {
  document.removeEventListener('mousedown', onDocMouseDown, true)
  document.removeEventListener('keydown', onDocKeydown)
  window.removeEventListener('scroll', onReposition, true)
  window.removeEventListener('resize', onReposition)
})

defineExpose({ openAt, close })
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      ref="panelEl"
      class="cp-pop"
      :style="{ left: `${pos.left}px`, top: `${pos.top}px` }"
      role="menu"
      @keydown="onKeydown"
    >
      <div v-for="group in groups" :key="group.title" class="cp-group">
        <p class="cp-group-title">{{ group.title }}</p>
        <div v-if="group.layout === 'grid'" class="cp-grid">
          <button
            v-for="entry in group.entries"
            :key="entry.key"
            type="button"
            class="cp-card"
            :class="[{ hl: flatIndexOf(entry.key) === highlight }]"
            role="menuitem"
            @click="pick(entry.key)"
            @mouseenter="highlight = flatIndexOf(entry.key)"
          >
            <span class="cp-badge" :class="`tone-${entry.badge}`">
              <Icon :name="entry.icon" :size="15" />
            </span>
            <span class="cp-card-label">{{ entry.label }}</span>
            <kbd v-if="entry.shortcut" class="cp-kbd">{{ entry.shortcut }}</kbd>
          </button>
        </div>
        <div v-else class="cp-list">
          <button
            v-for="entry in group.entries"
            :key="entry.key"
            type="button"
            class="cp-row"
            :class="[{ hl: flatIndexOf(entry.key) === highlight }]"
            role="menuitem"
            @click="pick(entry.key)"
            @mouseenter="highlight = flatIndexOf(entry.key)"
          >
            <span class="cp-badge sm" :class="`tone-${entry.badge}`">
              <Icon :name="entry.icon" :size="13" />
            </span>
            <span class="cp-row-label">{{ entry.label }}</span>
            <kbd v-if="entry.shortcut" class="cp-kbd">{{ entry.shortcut }}</kbd>
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.cp-pop {
  position: fixed;
  z-index: var(--z-toast);
  width: 360px;
  max-width: calc(100vw - 16px);
  padding: 10px 10px 6px;
  background: var(--bg-elevated);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow);
  animation: cp-in 120ms var(--ease);
}

.cp-group {
  padding: 2px 2px 8px;
}

.cp-group-title {
  margin: 0 0 6px;
  padding: 0 8px;
  font-size: var(--fs-xxs);
  font-weight: 600;
  letter-spacing: 0.06em;
  color: var(--text-3);
}

.cp-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 6px;
}

.cp-card {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  padding: 10px 10px;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  background: var(--bg-hover);
  color: var(--text-1);
  font-size: 13px;
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}
.cp-card:hover,
.cp-card.hl {
  background: var(--bg-active);
  border-color: var(--border-strong);
}
.cp-card:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}

.cp-card-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
  font-weight: 600;
}

.cp-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.cp-row {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  padding: 8px 10px;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-1);
  font-size: 12.5px;
  cursor: pointer;
  transition: background var(--dur) var(--ease);
}
.cp-row:hover,
.cp-row.hl {
  background: var(--bg-hover);
}
.cp-row:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: -1px;
}

.cp-row-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
}

/* 彩色图标徽章（深/浅主题变量，禁止硬编码色值） */
.cp-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 30px;
  height: 30px;
  border-radius: 8px;
}
.cp-badge.sm {
  width: 24px;
  height: 24px;
  border-radius: 6px;
}
.cp-badge.tone-blue {
  color: var(--info);
  background: color-mix(in srgb, var(--info) 14%, transparent);
}
.cp-badge.tone-amber {
  color: var(--warning);
  background: color-mix(in srgb, var(--warning) 14%, transparent);
}
.cp-badge.tone-green {
  color: var(--success);
  background: color-mix(in srgb, var(--success) 14%, transparent);
}
.cp-badge.tone-purple {
  color: var(--accent);
  background: var(--accent-tint);
}

.cp-kbd {
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

@keyframes cp-in {
  from {
    opacity: 0;
    transform: translateY(-4px) scale(0.98);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}
</style>
