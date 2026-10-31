/**
 * 根组件：全局一次性的 ToastHost（useToast 通知）与 ProgressBar（useProgress 顶部进度条）。
 * 视图通过 <router-view /> 渲染。
 * 另挂全局 error/unhandledrejection 监听：把未捕获的异常通过 console + toast 暴露，
 * 便于在没有 DevTools 的情况下定位前端问题。
 * 监听 macOS 原生菜单「About Miyo」事件以打开自定义关于弹窗。
 */
<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import ToastHost from './components/ToastHost.vue'
import ProgressBar from './components/ProgressBar.vue'
import Brand from './components/Brand.vue'
import AboutDialog from './components/AboutDialog.vue'
import ReleaseNotesModal from './components/ReleaseNotesModal.vue'
import { useToast } from './composables/useToast'
import { useLocaleStore } from './stores/locale'
import { startAutoUpdate, openAboutSignal, openReleaseSignal } from './composables/useAutoUpdate'

const toast = useToast()
const locale = useLocaleStore()
const t = locale.t
const route = useRoute()

/**
 * 悬浮品牌：仅在无路由匹配时展示。四个正式视图（项目列表 / 工作区 /
 * GraphQL / 实时调试）自带顶栏，在此叠加浮层会盖住返回按钮等头部控件。
 */
const showFloatingBrand = computed(() => route.matched.length === 0)
const showAbout = ref(false)
/** 独立版本更新详情弹窗（顶部更新 Tag / 设置更新卡片 / 更新 toast 动作打开）。 */
const showRelease = ref(false)
let unlistenAbout: UnlistenFn | null = null
let unlistenClose: UnlistenFn | null = null
let stopAutoUpdate: (() => void) | null = null

/** 设置更新行 → 打开关于弹窗（一键下载安装）。 */
watch(openAboutSignal, () => {
  showAbout.value = true
})

/** 顶部更新 Tag / 设置更新卡片 → 打开独立更新详情弹窗。 */
watch(openReleaseSignal, () => {
  showRelease.value = true
})

/** 同一消息短窗去重：连锁报错（error → unhandledrejection）只弹一次 toast。 */
const lastError = { key: '', at: 0 }

function reportGlobalError(message: string, error: unknown, titleKey: string): void {
  console.error('[global.error]', message, error)
  const now = Date.now()
  if (lastError.key === message && now - lastError.at < 3000) return
  lastError.key = message
  lastError.at = now
  toast.error(t(titleKey), { message, duration: 6000 })
}

/** 具名 handler：onBeforeUnmount 时对称移除，HMR / 单测反复挂载不残留监听。 */
function onWindowError(event: ErrorEvent): void {
  reportGlobalError(String(event.error?.message ?? event.message), event.error, 'app.pageError')
}

function onWindowRejection(event: PromiseRejectionEvent): void {
  const reason = event.reason
  reportGlobalError(
    reason instanceof Error ? reason.message : String(reason),
    reason,
    'app.unhandledError',
  )
}

onMounted(async () => {
  window.addEventListener('error', onWindowError)
  window.addEventListener('unhandledrejection', onWindowRejection)
  try {
    if ('__TAURI_INTERNALS__' in window) {
      unlistenAbout = await listen('miyo://about', () => {
        showAbout.value = true
      })
      // 定时检查更新：仅主窗口启动（实时调试弹出窗不重复检查），
      // 发现新版弹一次提醒，点击直达关于弹窗一键下载安装。
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      if (getCurrentWindow().label === 'main') {
        stopAutoUpdate = startAutoUpdate({
          onUpdateAvailable: ({ version }) => {
            toast.info(t('app.updateFound', { v: version }), {
              duration: 15000,
              action: {
                label: t('app.viewDetails'),
                run: () => {
                  showRelease.value = true
                },
              },
            })
          },
        })
      }
    }
  } catch {
    // 非 Tauri（浏览器预览）环境：忽略
  }

  // 关窗兜底：存在未保存草稿时先确认（dialog ask），防误触 ⌘W / 关闭按钮
  // 静默丢失编辑现场。确认放弃则 destroy() 直关（不再触发本回调）。
  try {
    if ('__TAURI_INTERNALS__' in window) {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      if (getCurrentWindow().label === 'main') {
        const { ask } = await import('@tauri-apps/plugin-dialog')
        unlistenClose = await getCurrentWindow().onCloseRequested(async (event) => {
          const { useWorkspaceStore } = await import('./stores/workspace')
          if (!useWorkspaceStore().hasDirtyDrafts()) return
          event.preventDefault()
          const quit = await ask(t('app.quitDirtyConfirm'), {
            title: 'Miyo',
            okLabel: t('app.quitDiscard'),
            cancelLabel: t('common.cancel'),
          })
          if (quit) void getCurrentWindow().destroy()
        })
      }
    }
  } catch {
    // 非 Tauri 或插件不可用：跳过兜底（页签/草稿已持久化，重启可恢复）
  }
})

onBeforeUnmount(() => {
  window.removeEventListener('error', onWindowError)
  window.removeEventListener('unhandledrejection', onWindowRejection)
  unlistenAbout?.()
  unlistenClose?.()
  stopAutoUpdate?.()
})
</script>

<template>
  <div v-if="showFloatingBrand" class="app-brand" :aria-label="t('app.brandAria')">
    <Brand title="Miyo" :subtitle="t('app.tagline')" />
  </div>
  <ProgressBar />
  <ToastHost />
  <AboutDialog v-model:open="showAbout" />
  <ReleaseNotesModal v-model:open="showRelease" />
  <main class="miyo-app">
    <router-view />
  </main>
</template>

<style scoped>
.app-brand {
  position: fixed;
  top: 8px;
  left: 8px;
  z-index: 60;
  padding: 2px;
  background: var(--bg-elevated);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow);
}
.app-brand :deep(.brand) {
  min-width: 0;
  width: 132px;
}
</style>