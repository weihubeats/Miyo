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

const pinia = createPinia()

// 品牌更名（RustFox → Miyo）的 localStorage 旧键迁移：必须先于任何读偏好键的
// store 初始化（主题 / 语言），否则老用户升级后偏好回落默认值。
migrateLegacyStorageKeys()

// 防闪烁（FOUC）：主题同步生效；语言字典按需（en 动态 import），在挂载前补齐。
useThemeStore(pinia).init()
void bootstrap()

async function bootstrap(): Promise<void> {
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
}
