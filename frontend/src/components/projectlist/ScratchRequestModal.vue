<script setup lang="ts">
/**
 * ScratchRequestModal：快速请求（暂存区）弹窗。
 *
 * - 不落库的临时请求：方法 + URL + 可选请求体；
 * - 展示状态码 / 耗时 / 大小 / 响应体（不写入历史）。
 */
import { ref, watch } from 'vue'
import Modal from '../ui/Modal.vue'
import CustomSelect from '../ui/CustomSelect.vue'
import Icon from '../ui/Icon.vue'
import { useMiyoApi } from '../../composables/useMiyoApi'
import { useToast } from '../../composables/useToast'
import { useLocaleStore } from '../../stores/locale'
import { formatBytes, formatDuration } from '../../utils/format'
import type { BodySpec, ExecuteResponse, HttpMethod } from '../../types/miyoApi'

const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const open = defineModel<boolean>('open', { default: false })

const scratchMethod = ref<HttpMethod>('GET')
const scratchUrl = ref('')
const scratchBody = ref('')
const scratchSending = ref(false)
const scratchRes = ref<ExecuteResponse | null>(null)
const SCRATCH_METHODS: Array<{ value: HttpMethod; label: string }> = [
  { value: 'GET', label: 'GET' },
  { value: 'POST', label: 'POST' },
  { value: 'PUT', label: 'PUT' },
  { value: 'PATCH', label: 'PATCH' },
  { value: 'DELETE', label: 'DELETE' },
]

// 每次打开时重置暂存区
watch(open, (v) => {
  if (v) {
    scratchUrl.value = ''
    scratchBody.value = ''
    scratchRes.value = null
  }
})

async function sendScratch(): Promise<void> {
  const url = scratchUrl.value.trim()
  if (!url) {
    toast.warning(t('scratch.urlRequired'))
    return
  }
  scratchSending.value = true
  scratchRes.value = null
  try {
    let body: BodySpec = { mode: 'none' }
    if (scratchMethod.value !== 'GET' && scratchBody.value.trim()) {
      const raw = scratchBody.value.trim()
      try {
        JSON.parse(raw)
        body = { mode: 'json', raw }
      } catch {
        body = { mode: 'text', raw }
      }
    }
    scratchRes.value = await api.executeRequest({
      url,
      method: scratchMethod.value,
      spec: {
        params: [],
        headers: [],
        path_variables: [],
        auth: { type: 'none' },
        body,
        timeout_ms: null,
        follow_redirects: true,
        tests: null,
      },
      environment_id: null,
    })
  } catch (e) {
    toast.error(t('scratch.sendFail'), { message: e instanceof Error ? e.message : String(e), duration: 6000 })
  } finally {
    scratchSending.value = false
  }
}
</script>

<template>
  <Modal v-model:open="open" :title="t('scratch.title')" width="580px" @close="open = false">
    <div class="scratch-row">
      <CustomSelect
        v-model="scratchMethod"
        :options="SCRATCH_METHODS"
        size="sm"
        class="scratch-method"
      />
      <input
        v-model="scratchUrl"
        class="miyo-input scratch-url"
        placeholder="https://api.example.com/posts"
        spellcheck="false"
        @keyup.enter="sendScratch"
      />
      <button
        class="miyo-btn miyo-btn-primary"
        type="button"
        :disabled="scratchSending"
        @click="sendScratch"
      >
        <Icon name="send" :size="13" /> {{ scratchSending ? t('scratch.sending') : t('editor.send') }}
      </button>
    </div>
    <textarea
      v-if="scratchMethod !== 'GET'"
      v-model="scratchBody"
      class="miyo-input scratch-ta"
      :placeholder="t('scratch.bodyPh')"
      spellcheck="false"
    ></textarea>
    <div v-if="scratchRes" class="scratch-res">
      <div class="scratch-res-top">
        <span class="sr-status" :class="{ ok: scratchRes.status < 400, err: scratchRes.status >= 400 }">
          {{ scratchRes.status }}
        </span>
        <span class="sr-meta"><Icon name="clock" :size="11" /> {{ formatDuration(scratchRes.duration_ms) }}</span>
        <span class="sr-meta"><Icon name="download" :size="11" /> {{ formatBytes(scratchRes.size_bytes) }}</span>
      </div>
      <pre class="sr-body">{{ scratchRes.body }}</pre>
    </div>
    <p v-else class="miyo-hint scratch-hint">{{ t('scratch.resHint') }}</p>
  </Modal>
</template>

<style scoped>
/* ---------- 快速请求暂存区 ---------- */
.scratch-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.scratch-method {
  width: 96px;
  flex-shrink: 0;
}

.scratch-url {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: 12.5px;
}

.scratch-ta {
  width: 100%;
  min-height: 90px;
  margin-top: 8px;
  font-family: var(--font-mono);
  font-size: 12.5px;
  resize: vertical;
}

.scratch-res {
  margin-top: 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-panel);
  overflow: hidden;
}

.scratch-res-top {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 7px 12px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-card);
}

.sr-status {
  padding: 1px 9px;
  border-radius: 999px;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  font-weight: 700;
}
.sr-status.ok {
  background: var(--success-tint);
  color: var(--success);
}
.sr-status.err {
  background: var(--danger-tint);
  color: var(--danger);
}

.sr-meta {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.sr-body {
  margin: 0;
  max-height: 240px;
  overflow: auto;
  padding: 10px 12px;
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 1.55;
  color: var(--text-1);
  white-space: pre-wrap;
  word-break: break-all;
}

.scratch-hint {
  margin-top: 10px;
}
</style>
