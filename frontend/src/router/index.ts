/**
 * 视图路由：与 Tauri 壳共享同一 origin，history 模式足够（SPA 单窗口）。
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
