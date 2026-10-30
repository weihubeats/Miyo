<script setup lang="ts">
/**
 * HeaderUpdateTag：顶部导航更新入口（设置齿轮左侧）。
 * - 检出新版本时常驻轻量胶囊 Tag（微亮火箭图标 + 更新 vX），点击打开独立更新
 *   详情弹窗 ReleaseNotesModal；安装 / 跳过 / 取走暂存后自动消失；
 * - 悬停（180ms 延迟）在按钮下方弹出本版更新日志预览卡片（Markdown 渲染，可滚动），
 *   移开（120ms 宽限）自动收起——卡片可移入滚动，不必先点开弹窗；
 * - 卡片 Teleport 到 body 并 fixed 定位（脱离顶栏 overflow 裁剪与堆叠上下文），
 *   贴按钮右缘向内夹进视口；点开弹窗 / 更新消失时立即收起。
 */
import { onBeforeUnmount, ref, watch } from 'vue'
import {
  hasPendingUpdate,
  pendingUpdateDate,
  pendingUpdateNotes,
  pendingUpdateVersion,
  requestOpenRelease,
} from '../composables/useAutoUpdate'
import { useLocaleStore } from '../stores/locale'
import ReleaseNotesBody from './ReleaseNotesBody.vue'

const t = useLocaleStore().t

/** 预览卡片宽度与触发区下方呼吸间距。 */
const CARD_WIDTH = 340
const CARD_GAP = 8
/** 悬停宽限：出现略迟（划过不误触），收起留痕（便于移入卡片滚动日志）。 */
const SHOW_DELAY_MS = 180
const HIDE_DELAY_MS = 120
/** 卡片最小/最大日志可视高度（窗口偏矮时随剩余空间收缩）。 */
const NOTES_MIN_H = 120
const NOTES_MAX_H = 260

const triggerEl = ref<HTMLElement | null>(null)
const open = ref(false)
const cardWidth = ref(CARD_WIDTH)
const pos = ref({ left: 0, top: 0 })
const notesMaxHeight = ref(`${NOTES_MAX_H}px`)

/** 卡片内容（打开时从共享暂存快照，不取走暂存——点开弹窗仍能接管同一份）。 */
const version = ref('')
const notes = ref('')
const dateText = ref('')

let showTimer: number | null = null
let hideTimer: number | null = null

/** RFC 3339 → 本地日期文本（服务器未提供或解析失败时为空 → 不渲染该行）。 */
function formatDate(raw: string | null): string {
  if (!raw) return ''
  const date = new Date(raw)
  return Number.isNaN(date.getTime()) ? '' : date.toLocaleDateString()
}

/** 定位：贴触发按钮右缘（按钮靠近窗口右侧），再夹进视口；日志高度按剩余空间收缩。 */
function measure(): void {
  const el = triggerEl.value
  if (!el) return
  const rect = el.getBoundingClientRect()
  const width = Math.min(CARD_WIDTH, window.innerWidth - 16)
  const top = rect.bottom + CARD_GAP
  cardWidth.value = width
  pos.value = {
    left: Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8)),
    top,
  }
  notesMaxHeight.value = `${Math.max(NOTES_MIN_H, Math.min(NOTES_MAX_H, window.innerHeight - top - 96))}px`
}

function clearShowTimer(): void {
  if (showTimer !== null) {
    window.clearTimeout(showTimer)
    showTimer = null
  }
}

function clearHideTimer(): void {
  if (hideTimer !== null) {
    window.clearTimeout(hideTimer)
    hideTimer = null
  }
}

function scheduleShow(): void {
  clearHideTimer()
  if (open.value) return
  clearShowTimer()
  showTimer = window.setTimeout(() => {
    showTimer = null
    // 悬停等待期间更新可能已装 / 被跳过：不再展示
    if (!hasPendingUpdate.value || !triggerEl.value) return
    version.value = pendingUpdateVersion() ?? ''
    notes.value = pendingUpdateNotes()
    dateText.value = formatDate(pendingUpdateDate())
    measure()
    open.value = true
  }, SHOW_DELAY_MS)
}

function scheduleHide(): void {
  clearShowTimer()
  if (!open.value) return
  clearHideTimer()
  hideTimer = window.setTimeout(() => {
    hideTimer = null
    open.value = false
  }, HIDE_DELAY_MS)
}

/** 鼠标移入卡片：撤销待收起（日志可长时间阅读 / 滚动）。 */
function keepOpen(): void {
  clearHideTimer()
}

function hideNow(): void {
  clearShowTimer()
  clearHideTimer()
  open.value = false
}

/** 点击：先收起预览再打开详情弹窗（避免卡片浮在遮罩之上）。 */
function onClick(): void {
  hideNow()
  requestOpenRelease()
}

/** 窗口尺寸变化后卡片位置会漂：正在展示时重新测一次。 */
function onWindowResize(): void {
  if (open.value) measure()
}

watch(hasPendingUpdate, (hasPending) => {
  // 更新被安装 / 跳过 / 取走：触发区消失，Teleport 出去的卡片须一并收起
  if (!hasPending) hideNow()
})

window.addEventListener('resize', onWindowResize)

onBeforeUnmount(() => {
  hideNow()
  window.removeEventListener('resize', onWindowResize)
})
</script>

<template>
  <button
    v-if="hasPendingUpdate"
    ref="triggerEl"
    class="miyo-update-tag flex cursor-pointer items-center gap-1.5 rounded-full border border-purple-500/40 bg-purple-500/20 px-3 py-1 font-mono text-xs font-medium text-purple-700 transition-all hover:bg-purple-500/30 dark:text-purple-300"
    type="button"
    :aria-label="t('app.updateFound', { v: pendingUpdateVersion() ?? '' })"
    @click="onClick"
    @mouseenter="scheduleShow"
    @mouseleave="scheduleHide"
  >
    <span class="miyo-update-tag-icon" aria-hidden="true">🚀</span>
    <span>{{ t('app.updateTag', { v: pendingUpdateVersion() ?? '' }) }}</span>
  </button>

  <!-- 悬停预览：本版更新日志（内容与详情弹窗同源同排版，仅高度/宽度不同） -->
  <Teleport to="body">
    <div
      v-if="open"
      class="miyo-update-card"
      :style="{ left: `${pos.left}px`, top: `${pos.top}px`, width: `${cardWidth}px` }"
      @mouseenter="keepOpen"
      @mouseleave="scheduleHide"
    >
      <div class="miyo-update-card-head">
        <span class="miyo-update-card-icon" aria-hidden="true">🚀</span>
        <span class="miyo-update-card-title">{{ t('app.updateFound', { v: version }) }}</span>
        <span v-if="dateText" class="miyo-update-card-date">{{ dateText }}</span>
      </div>
      <ReleaseNotesBody :notes="notes" :max-height="notesMaxHeight" />
    </div>
  </Teleport>
</template>

<style scoped>
/* 投影走 primary 光晕（原 shadow-purple-950/50 浅色下糊成暗紫脏影） */
.miyo-update-tag {
  box-shadow: 0 1px 3px var(--primary-glow);
}

/* 火箭微亮呼吸：prefers-reduced-motion 由 style.css 全局兜底关闭 */
.miyo-update-tag-icon {
  font-size: var(--fs-xxs);
  line-height: 1;
  animation: miyo-update-tag-glow 1.8s ease-in-out infinite;
}

@keyframes miyo-update-tag-glow {
  0%,
  100% {
    transform: scale(1);
    opacity: 1;
  }
  50% {
    transform: scale(1.2);
    opacity: 0.7;
  }
}

/* 悬停预览卡片：配色全走主题令牌（深/浅各一套），层级同 Tooltip / Menu
   （z-index 300），高过顶栏与弹窗，且不会被顶栏 overflow 裁剪（fixed + Teleport） */
.miyo-update-card {
  position: fixed;
  z-index: 300;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 14px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-lg);
  background: var(--bg-elevated);
  box-shadow: var(--shadow-lg);
  animation: miyo-update-card-in 140ms var(--ease);
  transform-origin: top right;
}

.miyo-update-card-head {
  display: flex;
  align-items: center;
  gap: 6px;
}

.miyo-update-card-icon {
  font-size: 12px;
  line-height: 1;
}

.miyo-update-card-title {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  font-weight: 700;
  color: var(--text-1);
}

/* 发布日期靠右靠后（服务器未提供时不渲染） */
.miyo-update-card-date {
  margin-left: auto;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

@keyframes miyo-update-card-in {
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
