import { fileURLToPath, URL } from 'node:url'

import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  // @ 别名 → src：domains 分层后组件跨层级引用统一走 @/，避免 ../../.. 链
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  // Tauri 开发模式固定端口：tauri.conf.json 的 devUrl 指向 5173
  server: {
    port: 5173,
    strictPort: true,
  },
  // Tauri 构建要求使用相对路径（产物经由 tauri://localhost 加载）
  base: './',
  clearScreen: false,
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: {
    rolldownOptions: {
      output: {
        /**
         * 首屏分包：CodeMirror 全家桶 + chart.js 原来全进主 chunk。
         * vue / codemirror / chart 拆独立 chunk，配合路由懒加载，
         * 首屏只下发框架 + 当前路由代码。
         * Vite 8（rolldown）弃用 manualChunks 对象形式，改用 codeSplitting.groups，
         * test 按包路径匹配，包清单与原 manualChunks 保持一致。
         */
        codeSplitting: {
          groups: [
            {
              name: 'vendor_vue',
              test: /[\\/]node_modules[\\/](vue|vue-router|pinia)[\\/]/,
            },
            {
              name: 'vendor_codemirror',
              test: /[\\/]node_modules[\\/]@codemirror[\\/](state|view|commands|language|lint|autocomplete|lang-json)[\\/]/,
            },
            {
              name: 'vendor_chart',
              test: /[\\/]node_modules[\\/](chart\.js|vue-chartjs)[\\/]/,
            },
          ],
        },
      },
    },
  },
})
