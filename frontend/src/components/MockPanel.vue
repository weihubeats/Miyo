<script setup lang="ts">
/**
 * MockPanel：Mock 管理（接口页内嵌）。
 * - 展示项目级 Mock 规则，高亮与当前接口关联的规则；
 * - 「打开 Mock 管理」复用完整 MockRuleDialog（新建 / 编辑 / 删除）。
 */
import { onMounted, ref } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import { methodTextTone } from '../utils/methodTone'
import type { Endpoint, MockRule } from '../types/miyoApi'
import EmptyState from './ui/EmptyState.vue'
import Icon from './ui/Icon.vue'

const props = defineProps<{ draft: Endpoint | null }>()
const emit = defineEmits<{ openManager: [] }>()

const store = useWorkspaceStore()
const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const rules = ref<MockRule[]>([])
const reloading = ref(false)

async function load(): Promise<void> {
  if (!store.project) return
  try {
    rules.value = (await api.listMockRules(store.project.id)) ?? []
  } catch {
    rules.value = []
  }
}

onMounted(load)

/** 热重载：运行中原子替换定义，无需重启服务（未运行会提示先启动）。 */
async function reload(): Promise<void> {
  if (reloading.value) return
  reloading.value = true
  try {
    const n = await api.mockReload()
    toast.success(t('mockpanel.reloaded', { n }))
  } catch (err) {
    toast.error(t('mockpanel.reloadFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    reloading.value = false
  }
}

/** 与当前接口关联的规则（endpoint_id 匹配）。 */
const related = (): MockRule[] =>
  props.draft ? rules.value.filter((r) => r.endpoint_id === props.draft?.id) : []

const others = (): MockRule[] =>
  props.draft ? rules.value.filter((r) => r.endpoint_id !== props.draft?.id) : []
</script>

<template>
  <div class="mkp">
    <div class="mkp-bar">
      <span class="mkp-title">{{ t('mockpanel.title', { n: rules.length }) }}</span>
      <span class="mkp-actions">
        <button
          class="miyo-btn miyo-btn-sm"
          type="button"
          :disabled="reloading"
          :title="t('mockpanel.reloadHint')"
          @click="reload"
        >
          <Icon name="refresh" :size="13" /> {{ reloading ? t('mockpanel.reloading') : t('mockpanel.reload') }}
        </button>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="emit('openManager')">
          <Icon name="settings" :size="13" /> {{ t('mockpanel.openManager') }}
        </button>
      </span>
    </div>

    <div v-if="related().length" class="mkp-sec">
      <span class="mkp-sec-label">{{ t('mockpanel.thisEndpoint') }}</span>
      <div class="mkp-list">
        <div v-for="r in related()" :key="r.id" class="mkp-row">
          <span class="mkp-method" :class="methodTextTone(r.method)">{{ r.method }}</span>
          <code class="mkp-path">{{ r.path }}</code>
          <span class="mkp-name">{{ r.name }}</span>
          <span class="mkp-status" :class="{ on: r.enabled }">{{ r.enabled ? t('mockpanel.enabled') : t('mockpanel.disabled') }}</span>
          <span class="mkp-code">{{ r.response_status }}</span>
        </div>
      </div>
    </div>

    <div v-if="others().length" class="mkp-sec">
      <span class="mkp-sec-label">{{ t('mockpanel.otherRules') }}</span>
      <div class="mkp-list">
        <div v-for="r in others()" :key="r.id" class="mkp-row">
          <span class="mkp-method" :class="methodTextTone(r.method)">{{ r.method }}</span>
          <code class="mkp-path">{{ r.path }}</code>
          <span class="mkp-name">{{ r.name }}</span>
          <span class="mkp-status" :class="{ on: r.enabled }">{{ r.enabled ? t('mockpanel.enabled') : t('mockpanel.disabled') }}</span>
        </div>
      </div>
    </div>

    <EmptyState
      v-if="!rules.length"
      icon="server"
      compact
      :title="t('mockpanel.empty')"
      :description="t('mockpanel.emptyHint')"
    />
  </div>
</template>

<style scoped>
.mkp {
  display: flex;
  flex-direction: column;
  gap: 12px;
  max-width: 720px;
}

.mkp-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.mkp-actions {
  display: inline-flex;
  gap: 8px;
}

.mkp-title {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-2);
}

.mkp-sec {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.mkp-sec-label {
  font-size: var(--fs-xs);
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.4px;
}

.mkp-list {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
}

.mkp-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 10px;
  border-bottom: 1px solid var(--border);
  font-size: 12.5px;
}
.mkp-row:last-child {
  border-bottom: none;
}

.mkp-method {
  flex-shrink: 0;
  width: 46px;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 700;
}

.mkp-path {
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-1);
}

.mkp-name {
  flex: 1;
  min-width: 0;
  color: var(--text-2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.mkp-status {
  flex-shrink: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.mkp-status.on {
  color: var(--ok);
}

.mkp-code {
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-3);
}
</style>