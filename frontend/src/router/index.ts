/**
 * 视图路由：与 Tauri 壳共享同一 origin，history 模式足够（SPA 单窗口）。
 *
 * 核心路由在此静态声明（懒加载组件）；插件路由由 PluginManager 在插件激活时
 * 动态注册（name 以 `plugin-` 开头，meta.pluginId 标记归属）——卸载 / 停用插件
 * 后其路由随之注销，core 路由不受影响。
 */
import { createRouter, createWebHistory } from 'vue-router'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/',
      redirect: '/projects',
    },
    {
      path: '/projects',
      name: 'projects',
      component: () => import('@/domains/workspace/views/ProjectList.vue'),
    },
    {
      path: '/graphql',
      name: 'graphql',
      component: () => import('@/domains/endpoint/views/GraphQLView.vue'),
    },
    {
      path: '/realtime',
      name: 'realtime',
      component: () => import('@/domains/endpoint/views/RealtimeView.vue'),
    },
    {
      path: '/integration',
      name: 'integration',
      component: () => import('@/domains/integration/views/IntegrationView.vue'),
    },
    {
      // 插件管理中心（核心能力，非插件）
      path: '/plugins',
      name: 'plugins',
      component: () => import('@/domains/plugins/views/PluginsView.vue'),
    },
    {
      path: '/ai-test',
      name: 'ai-test',
      component: () => import('@/domains/agent/views/AiTestView.vue'),
    },
    {
      path: '/workspace',
      name: 'workspace',
      component: () => import('@/domains/workspace/views/WorkspaceView.vue'),
    },
  ],
})

export default router
