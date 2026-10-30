<script setup lang="ts">
/**
 * AboutDialog：自定义「关于 Miyo」弹窗（替代系统默认 About 面板）。
 * - 品牌 logo（狐狸图标 + 渐变底 + 柔和投影）；
 * - 名称 / 版本 / 副标题 + GitHub / 检查更新链接 + 版权行；
 * - 检查更新（tauri-plugin-updater）：发现新版本 → 展示版本号与更新说明 →
 *   下载（带进度）→ 安装 → 重启（tauri-plugin-process relaunch）。
 * 触发来源：macOS 原生菜单「About Miyo」→ miyo://about 事件 → App.vue 打开。
 */
import { check, type Update } from '@tauri-apps/plugin-updater'
import { openUrl } from '@tauri-apps/plugin-opener'
import { ref, watch } from 'vue'
import { version } from '../../package.json'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import { useUpdateDownload } from '../composables/useUpdateDownload'
import { takePendingUpdate, restorePendingUpdate, skipUpdateVersion } from '../composables/useAutoUpdate'
import Icon from './ui/Icon.vue'
import Modal from './ui/Modal.vue'
import logo from '../assets/miyo-logo.svg'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ 'update:open': [open: boolean] }>()

const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const GITHUB_URL = 'https://github.com/weihubeats/Miyo'

/** 打开系统浏览器访问仓库（WebView 内 target=_blank 无效果，须走 opener 插件）。 */
async function openGitHub(): Promise<void> {
  try {
    await openUrl(GITHUB_URL)
  } catch (err) {
    toast.error(t('about.githubFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  }
}

const checking = ref(false)
/** 下载进度状态机：下载 / 安装中状态、进度条文案与失败重试（与更新详情弹窗共用）。 */
const {
  downloading,
  progress,
  installing,
  failMsg,
  statusText: downloadStatus,
  start,
} = useUpdateDownload()
/** 待安装新版本的展示信息（Update 对象含 JS 私有字段，不能进响应式 ref——
 *  Vue 的 Proxy 会让私有字段访问抛 "Cannot read private member"，故只存字符串）。 */
const pendingVersion = ref<string | null>(null)
const pendingNotes = ref('')
let pending: Update | null = null

/** 在系统浏览器打开当前待装版本的完整 Release Notes。 */
async function openReleaseNotes(): Promise<void> {
  if (!pendingVersion.value) return
  try {
    await openUrl(`${GITHUB_URL}/releases/tag/v${pendingVersion.value}`)
  } catch (err) {
    toast.error(t('about.notesFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  }
}

/**
 * 承接定时检查发现的更新：弹窗打开时若有暂存，直接展示版本号与说明，
 * 免二次检查，一键下载安装。
 */
watch(
  () => props.open,
  (open) => {
    if (!open) {
      // 关闭未安装：放回暂存（小红点重新点亮，下次打开继续承接）。
      // 展示态必须一并清空——否则重开时 `pendingVersion` 早退守卫先命中，
      // takePendingUpdate 永远不执行：卡片渲染着、Update 对象却在共享暂存里，
      // 点下载走 `!update` 静默 return（回归锁定，见 spec「关闭重开仍可下载」）。
      if (pendingVersion.value && pending) {
        restorePendingUpdate(pending)
        pending = null
      }
      pendingVersion.value = null
      pendingNotes.value = ''
      failMsg.value = ''
      return
    }
    if (pendingVersion.value) return
    const auto = takePendingUpdate()
    if (auto?.available) {
      pending?.close()
      pending = auto
      pendingVersion.value = auto.version
      pendingNotes.value = auto.body ?? ''
    } else {
      auto?.close()
    }
  },
)

async function checkUpdates(): Promise<void> {
  if (checking.value) return
  checking.value = true
  pendingVersion.value = null
  failMsg.value = ''
  pending?.close()
  pending = null
  try {
    const update = await check()
    if (update?.available) {
      pending = update
      pendingVersion.value = update.version
      pendingNotes.value = update.body ?? ''
    } else {
      update?.close()
      toast.info(t('about.upToDate', { v: version }))
    }
  } catch (err) {
    toast.error(t('about.checkFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    checking.value = false
  }
}

/**
 * 下载并安装当前待装版本（状态机见 useUpdateDownload）：
 * 成功后清本地展示状态（卡片收起），toast + 延迟重启由状态机负责。
 */
async function installUpdate(): Promise<void> {
  const update = pending
  if (!update || downloading.value) return
  const ok = await start(update)
  if (ok) {
    pending = null
    pendingVersion.value = null
  }
}

/** 跳过此版本：关闭待安装、记录跳过，不再提醒直到出现更新的版本；同时关闭本弹窗。 */
function skipVersion(): void {
  const v = pendingVersion.value
  if (!v || downloading.value) return
  pending?.close()
  pending = null
  pendingVersion.value = null
  pendingNotes.value = ''
  failMsg.value = ''
  skipUpdateVersion(v)
  toast.success(t('about.skipped', { v }), { message: t('about.skippedHint') })
  emit('update:open', false)
}
</script>

<template>
  <Modal
    :open="open"
    :title="t('about.title')"
    width="380px"
    @update:open="emit('update:open', $event)"
  >
    <div class="about">
      <div class="a-logo" aria-hidden="true">
        <img :src="logo" alt="" width="38" height="38" />
      </div>

      <div class="a-title">Miyo</div>
      <div class="a-version">v{{ version }}</div>
      <div class="a-subtitle">{{ t('about.subtitle') }}</div>

      <div class="a-links">
        <button class="a-link" type="button" @click="openGitHub">
          <Icon name="globe" :size="12" /> {{ t('about.github') }}
        </button>
        <span class="a-dot" aria-hidden="true"></span>
        <button class="a-link" type="button" :disabled="checking" @click="checkUpdates">
          <Icon name="refresh" :size="12" /> {{ checking ? t('about.checking') : t('about.check') }}
        </button>
      </div>

      <div v-if="pendingVersion" class="a-update">
        <div class="a-update-title">
          {{ t('about.newVersion') }} <span class="a-update-version">v{{ pendingVersion }}</span>
        </div>
        <button class="a-notes-link" type="button" @click="openReleaseNotes">
          {{ t('about.releaseNotes') }}
        </button>
        <p v-if="pendingNotes" class="a-update-notes">{{ pendingNotes }}</p>
        <template v-if="downloading">
          <div
            class="a-update-progress"
            role="progressbar"
            aria-valuemin="0"
            aria-valuemax="100"
            :aria-valuenow="progress == null ? undefined : Math.round(progress * 100)"
            :aria-valuetext="downloadStatus"
          >
            <div
              class="a-update-progress-bar"
              :style="{ width: progress == null ? '100%' : `${Math.round(progress * 100)}%` }"
              :class="{ indeterminate: progress == null && !installing }"
            ></div>
          </div>
          <span class="a-update-status">{{ downloadStatus }}</span>
          <span v-if="!installing" class="a-update-hint">{{ t('about.bgDownload') }}</span>
        </template>
        <template v-else>
          <p v-if="failMsg" class="a-update-error">{{ failMsg }}</p>
          <button class="a-update-btn" type="button" @click="installUpdate">
            {{ t('about.install') }}
          </button>
          <button class="a-skip-btn" type="button" @click="skipVersion">
            {{ t('about.skip') }}
          </button>
        </template>
        <span class="a-update-hint">{{ t('about.autoRestart') }}</span>
      </div>

      <div class="a-copyright">{{ t('about.copyright') }}</div>
    </div>
  </Modal>
</template>

<style scoped>
.about {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 6px 4px 2px;
  text-align: center;
}

.a-logo {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 64px;
  height: 64px;
  margin-bottom: 6px;
  color: #fff;
  border-radius: var(--radius-xl);
  background: var(--accent);
  box-shadow:
    0 10px 24px rgba(9, 12, 22, 0.45),
    inset 0 1px 0 rgba(255, 255, 255, 0.28);
}

.a-title {
  font-size: 18px;
  font-weight: 700;
  color: var(--text-1);
  letter-spacing: 0.2px;
}

.a-version {
  padding: 1px 9px;
  border-radius: 999px;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--accent);
  background: var(--accent-tint, rgba(168, 85, 247, 0.14));
}

.a-subtitle {
  margin-top: 4px;
  font-size: 12.5px;
  color: var(--text-2);
}

.a-links {
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 14px 0 4px;
}

.a-link {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border: none;
  background: none;
  padding: 0;
  font-family: inherit;
  font-size: 12.5px;
  color: var(--accent);
  text-decoration: none;
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    opacity var(--dur) var(--ease);
}
.a-link:hover {
  color: var(--accent-hover, var(--accent));
  opacity: 0.85;
}
.a-link:focus-visible {
  outline: 2px solid var(--focus-ring);
  outline-offset: 2px;
  border-radius: 4px;
}

.a-dot {
  width: 3px;
  height: 3px;
  border-radius: 50%;
  background: var(--text-3);
  opacity: 0.6;
}

.a-copyright {
  margin-top: 6px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.a-update {
  width: 100%;
  margin-top: 10px;
  padding: 12px;
  border: 1px solid var(--border, rgba(127, 127, 127, 0.25));
  border-radius: 10px;
  background: var(--bg-2, rgba(127, 127, 127, 0.08));
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
}

.a-update-title {
  font-size: 13px;
  color: var(--text-1);
}

.a-update-version {
  font-family: var(--font-mono);
  color: var(--accent);
}

.a-update-notes {
  max-height: 96px;
  overflow-y: auto;
  margin: 0;
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-2);
  white-space: pre-wrap;
  text-align: left;
}

.a-update-progress {
  width: 100%;
  height: 6px;
  border-radius: 999px;
  background: var(--bg-3);
  overflow: hidden;
}

.a-update-progress-bar {
  height: 100%;
  border-radius: 999px;
  background: var(--accent);
  transition: width 0.2s ease;
}

.a-update-progress-bar.indeterminate {
  animation: a-update-slide 1.2s ease-in-out infinite;
}

@keyframes a-update-slide {
  0% {
    transform: translateX(-100%);
  }
  100% {
    transform: translateX(100%);
  }
}

.a-update-status {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-2);
}

.a-notes-link {
  border: none;
  background: none;
  padding: 0;
  font-family: inherit;
  font-size: 12px;
  color: var(--accent);
  cursor: pointer;
  text-decoration: none;
  transition: color var(--dur) var(--ease);
}
.a-notes-link:hover {
  color: var(--accent-hover, var(--accent));
  text-decoration: underline;
}
.a-notes-link:focus-visible {
  outline: 2px solid var(--focus-ring);
  outline-offset: 2px;
  border-radius: 4px;
}

.a-update-hint {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.a-update-error {
  width: 100%;
  max-height: 56px;
  overflow-y: auto;
  margin: 0;
  font-size: 12px;
  line-height: 1.45;
  color: var(--danger, #f85149);
  text-align: left;
  word-break: break-all;
}

.a-update-btn {
  padding: 5px 16px;
  border: none;
  border-radius: 7px;
  font-family: inherit;
  font-size: 12.5px;
  color: #fff;
  background: var(--accent);
  cursor: pointer;
  transition: background var(--dur) var(--ease);
}

.a-update-btn:hover {
  background: var(--accent-hover, var(--accent));
}

.a-skip-btn {
  border: none;
  background: none;
  padding: 0;
  font-family: inherit;
  font-size: 12px;
  color: var(--text-3);
  cursor: pointer;
}
.a-skip-btn:hover {
  color: var(--text-1);
  text-decoration: underline;
}
</style>