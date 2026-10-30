<script setup lang="ts">
/**
 * ReleaseNotesModal：独立版本更新详情弹窗（顶部更新 Tag / 设置更新卡片的统一落地页）。
 * - Header：🚀 发现新版本 vX（加粗等宽，样式在文末非 scoped 段覆写 Modal 容器/标题）；
 * - Body：Markdown 更新日志（ReleaseNotesBody：markdownLite 转义渲染，主题化井底包裹框可滚动）；
 * - Footer：稍后再说（miyo-btn 次要按钮）+ 立即下载并重启（miyo-btn-primary 主按钮），下载中就地展示进度条；
 * - 打开时接管共享暂存的 Update；未安装关闭（Esc / 遮罩 / ✕ / 稍后再说）放回暂存，
 *   顶部 Tag 复亮；下载/安装中禁止关闭，成功后自动关闭并延迟重启。
 */
import { computed, ref, watch } from 'vue'
import type { Update } from '@tauri-apps/plugin-updater'
import { restorePendingUpdate, takePendingUpdate } from '../composables/useAutoUpdate'
import { useUpdateDownload } from '../composables/useUpdateDownload'
import { useLocaleStore } from '../stores/locale'
import Modal from './ui/Modal.vue'
import ReleaseNotesBody from './ReleaseNotesBody.vue'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ 'update:open': [open: boolean] }>()

const t = useLocaleStore().t

const { downloading, progress, installing, failMsg, statusText, start } = useUpdateDownload()

/** 待安装版本号与更新说明（打开时从共享暂存接管）。 */
const version = ref('')
const notes = ref('')
/** 本地接管的 Update 对象（关闭未安装时放回共享暂存）。 */
let held: Update | null = null

const title = computed(() => `🚀 ${t('app.updateFound', { v: version.value })}`)

watch(
  () => props.open,
  (open) => {
    if (!open) {
      if (held) {
        // 下载中不可达（guardClose 拦截）；未安装关闭 → 放回暂存，小红点复亮
        if (downloading.value) return
        restorePendingUpdate(held)
        held = null
      }
      version.value = ''
      notes.value = ''
      return
    }
    if (held) return
    const update = takePendingUpdate()
    if (update?.available) {
      held = update
      version.value = update.version
      notes.value = update.body ?? ''
    } else {
      // 暂存已被别处（关于弹窗）取走：无可展示内容，立即自关
      update?.close()
      emit('update:open', false)
    }
  },
)

/** 立即下载并重启：成功后清持有并自动关闭（状态机负责 toast + 延迟重启）。 */
async function install(): Promise<void> {
  if (!held || downloading.value) return
  const ok = await start(held)
  if (ok) {
    held = null
    emit('update:open', false)
  }
}

/** 稍后再说：关闭弹窗（watch 放回暂存，更新提醒保留）。 */
function later(): void {
  if (downloading.value) return
  emit('update:open', false)
}

/** Esc / 遮罩 / ✕ 关闭守卫：下载与安装中禁止关闭（进度就地可见）。 */
function guardClose(): boolean {
  return !downloading.value
}
</script>

<template>
  <Modal
    :open="open"
    :title="title"
    width="440px"
    dialog-class="rn-modal"
    mask-class="backdrop-blur-md"
    :guard-close="guardClose"
    @update:open="emit('update:open', $event)"
  >
    <!-- 固定高度 + 可滚动由 ReleaseNotesBody 承接；margin 留在本层（与 Modal 头部/底部对齐） -->
    <ReleaseNotesBody class="my-4" :notes="notes" />

    <template #footer>
      <div class="rn-foot">
        <div v-if="downloading" class="rn-progress-wrap">
          <div
            class="rn-progress"
            role="progressbar"
            aria-valuemin="0"
            aria-valuemax="100"
            :aria-valuenow="progress == null ? undefined : Math.round(progress * 100)"
            :aria-valuetext="statusText"
          >
            <div
              class="rn-progress-bar"
              :class="{ indeterminate: progress == null && !installing }"
              :style="{ width: progress == null ? '100%' : `${Math.round(progress * 100)}%` }"
            ></div>
          </div>
          <span class="rn-status">{{ statusText }}</span>
        </div>
        <p v-else-if="failMsg" class="rn-error" role="alert">{{ failMsg }}</p>
        <div class="rn-actions">
          <button class="miyo-btn" type="button" :disabled="downloading" @click="later">
            {{ t('release.later') }}
          </button>
          <button class="miyo-btn miyo-btn-primary" type="button" :disabled="downloading" @click="install">
            {{ t('release.install') }}
          </button>
        </div>
      </div>
    </template>
  </Modal>
</template>

<style scoped>
/* Footer：进度 / 错误 + 按钮行 */
.rn-foot {
  display: flex;
  flex-direction: column;
  gap: 8px;
  width: 100%;
}
.rn-progress-wrap {
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: 100%;
}
.rn-progress {
  width: 100%;
  height: 6px;
  overflow: hidden;
  border-radius: 999px;
  background: var(--bg-3);
}
.rn-progress-bar {
  height: 100%;
  border-radius: 999px;
  background: var(--accent);
  transition: width 0.2s ease;
}
.rn-progress-bar.indeterminate {
  animation: rn-slide 1.2s ease-in-out infinite;
}
@keyframes rn-slide {
  0% {
    transform: translateX(-100%);
  }
  100% {
    transform: translateX(100%);
  }
}
.rn-status {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-2);
}
.rn-error {
  margin: 0;
  font-size: 12px;
  line-height: 1.45;
  color: var(--danger, #f85149);
  word-break: break-all;
}
.rn-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>

<style>
/* Modal 遮罩/容器经 Teleport 挂到 body，覆写须非 scoped（同 SettingsDialog .sd-dialog 的变量先例）；
   类名 rn-modal 仅本弹窗使用，不会波及其他 Modal。
   选择器加 body 前缀保证特异性压过 Modal 自带的 scoped 规则（.m-xxx[data-v]）。
   配色一律不在此覆写：容器/标题/分隔线随主题令牌（--bg-elevated、--border、
   --text-*），遮罩走 Modal 自带 var(--mask)（深/浅各一套）——原「双主题恒深底」
   覆写在浅色下产出黑底弹窗，已移除。 */

/* 容器：仅覆写圆角（rounded 16px 结构性设计），颜色令牌全部回落主题 */
.rn-modal {
  --radius-lg: 16px;
}

/* p-6：内边距统一收到容器，内部三段归零 */
body .rn-modal.m-dialog {
  padding: 24px;
}
/* Header：flex items-center + justify-between（Modal .m-head 自带），pb-4 + 底分隔线，
   标题「🚀 发现新版本 vX」与右上 ✕ 垂直绝对居中对齐 */
body .rn-modal .m-head {
  padding: 0 0 16px;
  border-bottom: 1px solid var(--border);
}
body .rn-modal .m-body {
  padding: 0;
}
body .rn-modal .m-foot {
  /* 上方 hairline 由通用 .m-foot 提供，保留 12px 呼吸 */
  padding: 12px 0 0;
}

/* 标题：加粗等宽（文字随 --text-1 主题令牌），与右侧关闭 Icon 垂直对齐（m-head 已 flex center） */
body .rn-modal .m-title {
  font-family: var(--font-mono);
  font-size: 14px;
  font-weight: 700;
  color: var(--text-1);
}
</style>
