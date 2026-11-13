<script setup lang="ts">
/**
 * CodeExportDialog：接口代码导出弹窗。
 * 复用 CodePanel（语言选择 → 生成 → 复制），打开即按当前请求配置自动生成
 * curl / Python / JavaScript / Go / Java / PHP 代码片段。
 * 入参为 codegen 原始输入（与 CodeExportMenu 一致），调试页 / 用例抽屉通用。
 */
import CodePanel from '@/domains/endpoint/components/CodePanel.vue'
import Modal from '@/components/ui/Modal.vue'
import { useLocaleStore } from '@/stores/locale'
import type { AuthSpec, BodySpec, HttpMethod, KeyValue } from '@/types/miyoApi.d'

defineProps<{
  method: HttpMethod
  url: string
  headers: KeyValue[]
  body: BodySpec
  auth: AuthSpec
}>()
const emit = defineEmits<{ close: [] }>()

const locale = useLocaleStore()
const t = locale.t
</script>

<template>
  <Modal :open="true" :title="t('codeexport.title')" width="680px" @close="emit('close')">
    <p class="modal-hint">
      {{ t('codeexport.hint') }}
    </p>
    <CodePanel
      :method="method"
      :url="url"
      :headers="headers"
      :body="body"
      :auth="auth"
      auto-generate
    />
    <template #footer>
      <button class="miyo-btn miyo-btn-primary" type="button" @click="emit('close')">{{ t('common.close') }}</button>
    </template>
  </Modal>
</template>

<style scoped>
.modal-hint {
  margin: 0 0 10px;
  font-size: 12.5px;
  color: var(--text-2);
}
</style>