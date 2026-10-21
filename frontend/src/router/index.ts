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
      component: () => import('../views/ProjectList.vue'),
    },
    {
      path: '/graphql',
      name: 'graphql',
      component: () => import('../views/GraphQLView.vue'),
    },
    {
      path: '/realtime',
      name: 'realtime',
      component: () => import('../views/RealtimeView.vue'),
    },
    {
      path: '/workspace',
      name: 'workspace',
      component: () => import('../views/WorkspaceView.vue'),
    },
    {
      path: '/skills',
      name: 'skills',
      component: () => import('../views/SkillsView.vue'),
    },
  ],
})

export default router
