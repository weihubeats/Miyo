<script setup lang="ts">
/**
 * PluginIframeHost：第三方 UI 插件的沙箱宿主（插件路由的加载目标）。
 *
 * 流程：路由 meta.pluginId → PluginManager 注册表 → install_path →
 * read_text_file 读取 index.html → Blob URL → `sandbox="allow-scripts"`
 * iframe（opaque origin：插件无 DOM / 存储 / 父窗口访问）→ HostRpcBridge
 * 把 postMessage RPC 分发到插件 SDK 会话。
 *
 * 安全要点：
 * - iframe 不带 allow-same-origin：插件 JS 与主程序严格同源隔离；
 * - 插件文件只读（install 目录不经此宿主写入）；
 * - iframe 加载失败 / 插件缺失给出错误态与返回入口；
 * - 组件卸载即 dispose 桥（事件监听与会话回收）。
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import Icon from '@/components/ui/Icon.vue'
import { useLocaleStore } from '@/stores/locale'
import { useToast } from '@/composables/useToast'
import { usePluginManager } from '@/plugins/manager'
import { HostRpcBridge } from '@/plugins/host/bridge'
import type { RpcOutboundFrame } from '@/plugins/host/protocol'
import { errorText } from '@/utils/errorText'

const route = useRoute()
const router = useRouter()
const t = useLocaleStore().t
const toast = useToast()
const manager = usePluginManager()

const IPC_PREFIX = 'plugin:miyo|'

const pluginId = computed(() => (route.meta?.pluginId as string | undefined) ?? null)
const entry = computed(() => (pluginId.value ? manager.entries.get(pluginId.value) : undefined))

const htmlUrl = ref<string | null>(null)
const loadState = ref<'loading' | 'ready' | 'error'>('loading')
const loadError = ref('')
const iframeRef = ref<HTMLIFrameElement | null>(null)

let bridge: HostRpcBridge | null = null
let onMessage: ((event: MessageEvent) => void) | null = null

/** 装载插件页面并建立 RPC 桥。 */
async function loadPlugin(): Promise<void> {
  disposeBridge()
  htmlUrl.value = null
  const view = entry.value?.view
  if (!view || !view.enabled) {
    loadState.value = 'error'
    loadError.value = t('plugins.host.notEnabled')
    return
  }
  const installPath = view.install_path
  if (!installPath) {
    loadState.value = 'error'
    loadError.value = t('plugins.host.noInstallPath')
    return
  }
  loadState.value = 'loading'
  try {
    // 入口文件相对 install_path（manifest.entry 由后端校验为相对路径）
    const entryFile = view.manifest.entry ?? 'index.html'
    const html = await invoke<string>(`${IPC_PREFIX}read_text_file`, {
      path: `${installPath}/${entryFile}`,
    })
    // Blob URL：frame-src 'blob:' 已在 CSP 放行；沙箱属性见模板。
    htmlUrl.value = URL.createObjectURL(new Blob([html], { type: 'text/html' }))
    loadState.value = 'ready'
  } catch (e) {
    loadState.value = 'error'
    loadError.value = errorText(e)
  }
}

function disposeBridge(): void {
  if (onMessage) {
    window.removeEventListener('message', onMessage)
    onMessage = null
  }
  bridge?.dispose()
  bridge = null
  if (htmlUrl.value) {
    URL.revokeObjectURL(htmlUrl.value)
    htmlUrl.value = null
  }
}

/** iframe 就绪：建立桥 + 监听消息（源窗口严格校验）。 */
function onIframeLoad(): void {
  const frame = iframeRef.value
  const view = entry.value?.view
  if (!frame?.contentWindow || !view) return
  const session = manager.sessionOf(view.id)
  if (!session) {
    toast.error(t('plugins.host.noSession'))
    return
  }
  const target = frame.contentWindow
  bridge = new HostRpcBridge({
    sourceWindow: target,
    postMessage: (frameData: RpcOutboundFrame) => target.postMessage(frameData, '*'),
    session,
  })
  onMessage = (event: MessageEvent) => {
    // 只接受本宿主 iframe 的消息（沙箱 origin 是 opaque，不能按 origin 过滤）
    if (event.source !== target) return
    bridge?.handleMessage(event.data)
  }
  window.addEventListener('message', onMessage)
}

watch(
  () => [pluginId.value, entry.value?.runtime] as const,
  () => {
    if (route.path !== pluginRoutePath.value) return
    void loadPlugin()
  },
  { immediate: true },
)

/** 当前插件的主路由（iframe 页面对应的路由路径）。 */
const pluginRoutePath = computed(() => route.path)

onBeforeUnmount(disposeBridge)
</script>

<template>
  <div class="ph-root">
    <header class="ph-topbar">
      <Icon name="package" :size="15" />
      <span class="ph-title">
        {{ entry ? (entry.view.manifest.name_key ? t(entry.view.manifest.name_key) : entry.view.manifest.name) : (pluginId ?? '') }}
      </span>
      <span class="ph-badge">{{ t('plugins.host.sandboxBadge') }}</span>
      <div class="ph-spacer"></div>
      <button class="miyo-btn miyo-btn-sm" type="button" @click="router.push('/plugins')">
        {{ t('plugins.host.backToCenter') }}
      </button>
    </header>
    <main class="ph-body">
      <div v-if="loadState === 'loading'" class="ph-state">{{ t('plugins.host.loading') }}</div>
      <div v-else-if="loadState === 'error'" class="ph-state ph-error" role="alert">
        <p>{{ t('plugins.host.loadFailed') }}</p>
        <p class="ph-error-detail">{{ loadError }}</p>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="loadPlugin">
          {{ t('common.retry') }}
        </button>
      </div>
      <iframe
        v-else-if="htmlUrl"
        ref="iframeRef"
        class="ph-frame"
        :src="htmlUrl"
        sandbox="allow-scripts"
        :title="entry?.displayName ?? 'plugin'"
        @load="onIframeLoad"
      ></iframe>
    </main>
  </div>
</template>

<style scoped>
.ph-root {
  display: flex;
  flex-direction: column;
  height: 100vh;
  min-height: 0;
  background: var(--bg-app);
  color: var(--text-1);
}

.ph-topbar {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-panel);
  -webkit-app-region: drag;
}
.ph-topbar > * {
  -webkit-app-region: no-drag;
}
.ph-title {
  font-size: var(--fs-sm);
  font-weight: 600;
}
.ph-badge {
  font-size: var(--fs-2xxs);
  padding: 1px 6px;
  border-radius: var(--radius-pill);
  border: 1px solid color-mix(in srgb, var(--info) 50%, transparent);
  color: var(--info);
  background: color-mix(in srgb, var(--info) 10%, transparent);
}
.ph-spacer {
  flex: 1;
}

.ph-body {
  flex: 1;
  min-height: 0;
  display: flex;
}

.ph-frame {
  flex: 1;
  border: none;
  background: var(--bg-panel);
}

.ph-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  font-size: var(--fs-sm);
  color: var(--text-2);
}
.ph-error {
  color: var(--danger);
}
.ph-error-detail {
  font-size: var(--fs-xs);
  color: var(--text-3);
  max-width: 480px;
  overflow-wrap: anywhere;
  text-align: center;
}
</style>
