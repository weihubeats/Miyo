<script setup lang="ts">
/**
 * RequestDetail：场景结果行里的「请求参数」只读快照。
 *
 * 数据来自响应体的 `request`（Rust `RequestSnapshot`：变量渲染 / 认证注入 /
 * Cookie 回放之后的**实际发出形态**），与单请求流程同口径；旧后端缺快照时
 * 回退到结果自带的渲染后 `url`，只显示方法与地址。
 *
 * 折叠展示：方法 + 地址一行常显，请求头 / Cookie / 请求体默认收起——
 * 场景面板里一次要列多步，全展开会淹没响应与断言。
 */
import { computed, ref, watch } from 'vue'
import { useLocaleStore } from '../stores/locale'
import { isSensitiveVarName } from '../utils/scenarioVars'
import type { ScenarioStepResponse } from '../types/miyoApi'
import Icon from './ui/Icon.vue'

const props = defineProps<{
  /** 步骤响应快照（含 `request` 请求快照）。 */
  response: ScenarioStepResponse
  /** 结果自带的渲染后实际 URL（快照缺失时的兜底展示）。 */
  url?: string
}>()

const t = useLocaleStore().t

const request = computed(() => props.response.request ?? null)
const method = computed(() => request.value?.method ?? '')
/** 实际发出的完整地址（含 query）；快照缺失回退结果 URL。 */
const fullUrl = computed(() => request.value?.url ?? props.url ?? '')

const open = ref(false)
/** 请求体在场景面板里默认收起（可能是大 JSON）。 */
const bodyOpen = ref(false)

// 同一行换了内容（重跑 / 历史回看）时回到折叠态，避免残留上一条的展开状态。
watch(
  () => props.response,
  () => {
    open.value = false
    bodyOpen.value = false
  },
)

/** 头名命中敏感词根（Authorization / Cookie 等）时掩码 value。 */
function headerValue(name: string, value: string): string {
  return isSensitiveVarName(name) ? '••••••' : value
}

const hasDetails = computed(
  () => !!request.value && (request.value.headers.length > 0 || request.value.cookies.length > 0 || !!request.value.body),
)
</script>

<template>
  <section class="rqd">
    <div class="rqd-line">
      <Icon name="arrow-up-right" :size="11" class="rqd-glyph" />
      <span class="rqd-k">{{ t('scenario.reqSent') }}</span>
      <code class="rqd-method">{{ method }}</code>
      <code class="rqd-url" :title="fullUrl">{{ fullUrl }}</code>
      <button
        v-if="hasDetails"
        type="button"
        class="rqd-toggle"
        :aria-expanded="open"
        @click.stop="open = !open"
      >
        <Icon :name="open ? 'chevrons-down-up' : 'chevrons-up-down'" :size="12" />
        {{ t('scenario.reqParams') }}
      </button>
    </div>

    <div v-if="open && request" class="rqd-body">
      <p v-if="request.url_template && request.url_template !== request.url" class="rqd-row">
        <span class="rqd-k">{{ t('scenario.reqTemplate') }}</span>
        <code class="rqd-v">{{ request.url_template }}</code>
      </p>
      <div v-if="request.headers.length" class="rqd-row">
        <span class="rqd-k">{{ t('scenario.reqHeaders') }}</span>
        <ul class="rqd-list">
          <li v-for="([k, v], i) in request.headers" :key="`h-${i}`">
            <code>{{ k }}: {{ headerValue(k, v) }}</code>
          </li>
        </ul>
      </div>
      <div v-if="request.cookies.length" class="rqd-row">
        <span class="rqd-k">{{ t('scenario.reqCookies') }}</span>
        <ul class="rqd-list">
          <li v-for="([k, v], i) in request.cookies" :key="`c-${i}`">
            <code>{{ k }} = {{ headerValue(k, v) }}</code>
          </li>
        </ul>
      </div>
      <div v-if="request.body" class="rqd-row">
        <button type="button" class="rqd-k rqd-body-toggle" @click.stop="bodyOpen = !bodyOpen">
          <Icon :name="bodyOpen ? 'chevrons-down-up' : 'chevrons-up-down'" :size="12" />
          {{ t('scenario.reqBody') }} · {{ request.body.length }}
        </button>
        <pre v-if="bodyOpen" class="rqd-pre">{{ request.body }}</pre>
        <p v-if="request.body_omitted" class="rqd-note">{{ t('response.reqBodyTruncated') }}</p>
      </div>
    </div>
  </section>
</template>

<style scoped>
.rqd {
  display: flex;
  flex-direction: column;
  gap: 4px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-hover);
  padding: 6px 8px;
}

.rqd-line {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.rqd-glyph {
  flex: none;
  color: var(--text-3);
}

.rqd-k {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.rqd-method {
  flex: none;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-2);
}

.rqd-url {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rqd-toggle {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 3px;
  border: none;
  background: transparent;
  padding: 1px 4px;
  border-radius: var(--radius-sm);
  font-size: var(--fs-xxs);
  color: var(--accent);
  cursor: pointer;
}

.rqd-toggle:hover {
  background: var(--bg-card);
}

.rqd-body {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-top: 4px;
  border-top: 1px dashed var(--border);
}

.rqd-row {
  display: flex;
  gap: 8px;
  min-width: 0;
  margin: 0;
}

.rqd-v {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-2);
  word-break: break-all;
}

.rqd-list {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.rqd-list code {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-2);
  word-break: break-all;
}

.rqd-body-toggle {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  border: none;
  background: transparent;
  padding: 0;
  cursor: pointer;
}

.rqd-pre {
  margin: 0;
  flex: 1;
  min-width: 0;
  max-height: 200px;
  overflow: auto;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  line-height: 1.5;
  color: var(--text-1);
  white-space: pre-wrap;
  word-break: break-all;
}

.rqd-note {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--warning);
}
</style>