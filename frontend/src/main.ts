/**
 * 应用入口：装配 Pinia（状态）+ Router（视图路由）。
 * 全局 Toast / Progress 由 App.vue 挂载一次，勿在视图内重复实例化。
 */
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from '@/App.vue'
import router from '@/router'
import { useThemeStore } from '@/stores/theme'
import { useLocaleStore } from '@/stores/locale'
import { usePluginManager } from '@/plugins/manager'
import { migrateLegacyStorageKeys } from '@/utils/legacyStorage'
// 字体：仅 latin 子集 + 仅 woff2 + 400/500/700（原 8 个 fontsource 入口产出
// 44 个 woff/woff2 共 760KB，占 dist 约 1/3），声明见 fonts.css。
import './fonts.css'
import './style.css'
import tooltipOverflow from '@/directives/tooltipOverflow'
import focusEnd from '@/directives/focusEnd'

// macOS 无边框标题栏（Overlay）：标记平台，供全局 CSS 为顶部栏预留交通灯空间。
if (navigator.userAgent.includes('Mac')) {
  document.documentElement.setAttribute('data-platform', 'macos')
}

// 窗口标记：只有主窗（label 'main'）是 Overlay 标题栏、交通灯压在 Web 内容
// 左上角；实时调试 popout 等弹出子窗用原生标题栏，无需预留。label 与官方
// getCurrentWindow() 同源（__TAURI_INTERNALS__.metadata.currentWindow.label），
// 同步读避免首帧闪烁；浏览器预览无此内部对象，不设置。
const windowLabel = (
  window as unknown as {
    __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } }
  }
).__TAURI_INTERNALS__?.metadata?.currentWindow?.label
if (windowLabel) {
  document.documentElement.setAttribute('data-window', windowLabel)
}

const pinia = createPinia()

// 品牌更名（RustFox → Miyo）的 localStorage 旧键迁移：必须先于任何读偏好键的
// store 初始化（主题 / 语言），否则老用户升级后偏好回落默认值。
migrateLegacyStorageKeys()

// 防闪烁（FOUC）：主题同步生效；语言字典按需（en 动态 import），在挂载前补齐。
useThemeStore(pinia).init()
void bootstrap()

async function bootstrap(): Promise<void> {
  const boot = (window as unknown as { __MIYO_BOOT__?: { done?: () => void; fail?: (err: unknown) => void } }).__MIYO_BOOT__
  try {
    // 等启动语言字典就绪再挂载：首帧即完整文案，不回落另一语言。
    // （模块顶层 await 会撞 es2020 构建目标，故放这里。）
    await useLocaleStore(pinia).init()
    // 插件系统引导：拉取已安装插件并激活启用项（异步，不阻塞首屏挂载；
    // 激活完成前导航区不含插件项——按需加载，未启用插件零字节拉取）。
    void usePluginManager(pinia).boot()
    createApp(App)
      .use(pinia)
      .use(router)
      .directive('tooltip-overflow', tooltipOverflow)
      .directive('focus-end', focusEnd)
      .mount('#app')
    boot?.done?.()
    reportFrontendReady()
    startRenderHeartbeat()
  } catch (err) {
    console.error('[miyo.bootstrap]', err)
    boot?.fail?.(err)
  }
}

/**
 * 告知壳层「界面已真正挂载」：壳侧首帧看门狗超过 8 秒收不到这个事件就当黑屏，
 * 弹原生框给出 WebView2 / 硬件加速 / 日志路径三条指引（前端黑屏时用户点不到设置）。
 * 事件名与 `frontend/src-tauri/src/main.rs` 的常量保持一致；浏览器预览无 IPC 时静默跳过。
 */
function reportFrontendReady(): void {
  const internals = (
    window as unknown as {
      __TAURI_INTERNALS__?: { invoke?: (cmd: string, args?: unknown) => Promise<unknown> }
    }
  ).__TAURI_INTERNALS__
  if (!internals?.invoke) return
  void internals.invoke('plugin:event|emit', { event: 'miyo:frontend-ready', payload: null }).catch(() => {
    // 壳层未注册监听（旧版后端 / 纯前端调试）：不影响启动
  })
}

/**
 * 渲染心跳：`requestAnimationFrame` 每跑一帧就通知壳层一次（节流到约 1 秒）。
 *
 * 壳侧（`frontend/src-tauri/src/shell_guard.rs`）据此判断画面是否还在刷新。
 * 存在的意义：有些环境下页面加载成功、`frontend-ready` 准时到达，但
 * 合成器输出永不呈现到窗口——用户看到的是全黑，而「就绪」信号无法区分。
 * rAF 停止 = 帧不再被提交 = 画面冻结，这是唯一可靠的判断依据。
 *
 * 浏览器预览无 IPC 时静默跳过；`prefers-reduced-motion` 下 rAF 仍在跑，不受影响。
 */
function startRenderHeartbeat(): void {
  const internals = (
    window as unknown as {
      __TAURI_INTERNALS__?: { invoke?: (cmd: string, args?: unknown) => Promise<unknown> }
    }
  ).__TAURI_INTERNALS__
  if (!internals?.invoke) return
  const send = internals.invoke
  let last = 0
  const tick = (now: number): void => {
    if (now - last >= HEARTBEAT_INTERVAL_MS) {
      last = now
      // 不等结果、不处理错误：心跳丢失本身由壳侧超时判定，这里重试无意义。
      void send('render_heartbeat').catch(() => {})
    }
    requestAnimationFrame(tick)
  }
  requestAnimationFrame(tick)
}

/** 心跳节流间隔：壳侧看门狗按秒级判定停滞，1 秒一次足够且开销可忽略。 */
const HEARTBEAT_INTERVAL_MS = 1000
