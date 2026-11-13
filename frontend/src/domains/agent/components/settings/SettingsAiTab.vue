<script setup lang="ts">
/**
 * SettingsAiTab：AI 模型设置（OpenAI 兼容协议）。
 * Base URL / API Key / 模型名 + 预设下拉 + 连通性验活。
 * API Key 掩码回显；留空 = 保持原值（后端约定）。
 */
import { onMounted, ref } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import { errorText } from '@/utils/errorText'
import Icon from '@/components/ui/Icon.vue'
import type { AiSettingsView } from '@/types/miyoApi.d'

const t = useLocaleStore().t
const api = useMiyoApi()
const toast = useToast()

const loading = ref(true)
const saving = ref(false)
const testing = ref(false)
const settings = ref<AiSettingsView | null>(null)

const baseUrl = ref('')
const apiKey = ref('')
const model = ref('')
const temperature = ref<number | null>(null)
const maxTokens = ref<number | null>(null)

/** 预设（OpenAI 兼容端点惯例；可改任意值）。 */
const PRESETS = [
  { value: 'deepseek', label: 'DeepSeek', url: 'https://api.deepseek.com/v1', model: 'deepseek-chat' },
  { value: 'qwen', label: 'Qwen (DashScope)', url: 'https://dashscope.aliyuncs.com/compatible-mode/v1', model: 'qwen-plus' },
  { value: 'kimi', label: 'Kimi (Moonshot)', url: 'https://api.moonshot.cn/v1', model: 'moonshot-v1-32k' },
  { value: 'glm', label: 'GLM', url: 'https://open.bigmodel.cn/api/paas/v4', model: 'glm-4.6' },
  { value: 'openai', label: 'OpenAI', url: 'https://api.openai.com/v1', model: 'gpt-4o-mini' },
  { value: 'ollama', label: 'Ollama (local)', url: 'http://127.0.0.1:11434/v1', model: 'qwen2.5:14b' },
] as const
const preset = ref('')

function applyPreset(value: string): void {
  preset.value = value
  const p = PRESETS.find((x) => x.value === value)
  if (p) {
    baseUrl.value = p.url
    if (!model.value.trim() || PRESETS.some((x) => x.model === model.value)) {
      model.value = p.model
    }
  }
}

onMounted(async () => {
  try {
    settings.value = await api.getAiSettings()
    baseUrl.value = settings.value.base_url
    model.value = settings.value.model
    temperature.value = settings.value.temperature
    maxTokens.value = settings.value.max_tokens
    for (const p of PRESETS) {
      if (p.url === baseUrl.value) {
        preset.value = p.value
        break
      }
    }
  } catch (e) {
    toast.error(errorText(e))
  } finally {
    loading.value = false
  }
})

async function save(): Promise<void> {
  saving.value = true
  try {
    await api.setAiSettings({
      baseUrl: baseUrl.value.trim(),
      // 留空 = 保持原 Key（后端约定）；掩码态不回传。
      apiKey: apiKey.value.trim() ? apiKey.value.trim() : null,
      model: model.value.trim(),
      temperature: temperature.value,
      maxTokens: maxTokens.value,
    })
    settings.value = await api.getAiSettings()
    apiKey.value = ''
    toast.success(t('settings.ai.saved'))
  } catch (e) {
    toast.error(errorText(e))
  } finally {
    saving.value = false
  }
}

async function testConnection(): Promise<void> {
  testing.value = true
  try {
    // 先保存再验活（用未保存的草稿配置测没有意义）。
    await save()
    await api.aiTestConnection()
    toast.success(t('settings.ai.testOk'))
  } catch (e) {
    toast.error(errorText(e))
  } finally {
    testing.value = false
  }
}
</script>

<template>
  <div class="settings-ai">
    <section class="form-section">
      <h3 class="form-title">{{ t('settings.ai.title') }}</h3>
      <p class="form-hint">{{ t('settings.ai.hint') }}</p>

      <label class="form-label">{{ t('settings.ai.preset') }}</label>
      <select class="miyo-select" :value="preset" @change="applyPreset(($event.target as HTMLSelectElement).value)">
        <option value="">{{ t('settings.ai.presetCustom') }}</option>
        <option v-for="p in PRESETS" :key="p.value" :value="p.value">{{ p.label }}</option>
      </select>

      <label class="form-label">{{ t('settings.ai.baseUrl') }}</label>
      <input
        v-model="baseUrl"
        class="miyo-input"
        placeholder="https://api.deepseek.com/v1"
        spellcheck="false"
      />

      <label class="form-label">
        {{ t('settings.ai.apiKey') }}
        <span v-if="settings?.api_key_set" class="key-masked">
          {{ t('settings.ai.keySet', { mask: settings.api_key_masked }) }}
        </span>
      </label>
      <input
        v-model="apiKey"
        class="miyo-input"
        type="password"
        autocomplete="off"
        :placeholder="settings?.api_key_set ? t('settings.ai.keyKeep') : 'sk-…'"
        spellcheck="false"
      />

      <label class="form-label">{{ t('settings.ai.model') }}</label>
      <input v-model="model" class="miyo-input" placeholder="deepseek-chat" spellcheck="false" />

      <div class="form-row">
        <div class="form-col">
          <label class="form-label">{{ t('settings.ai.temperature') }}</label>
          <input
            class="miyo-input"
            type="number"
            min="0"
            max="2"
            step="0.1"
            :value="temperature ?? ''"
            @input="temperature = (($event.target as HTMLInputElement | null)?.value ?? '') === '' ? null : Number(($event.target as HTMLInputElement).value)"
          />
        </div>
        <div class="form-col">
          <label class="form-label">{{ t('settings.ai.maxTokens') }}</label>
          <input
            class="miyo-input"
            type="number"
            min="1"
            :value="maxTokens ?? ''"
            @input="maxTokens = (($event.target as HTMLInputElement | null)?.value ?? '') === '' ? null : Number(($event.target as HTMLInputElement).value)"
          />
        </div>
      </div>

      <div class="form-actions">
        <button class="miyo-btn miyo-btn-primary" :disabled="saving || loading" @click="save">
          <Icon name="save" :size="13" />
          {{ t('common.save') }}
        </button>
        <button class="miyo-btn" :disabled="testing || loading" @click="testConnection">
          <Icon name="plug" :size="13" />
          {{ t('settings.ai.test') }}
        </button>
      </div>

      <p class="form-note">{{ t('settings.ai.note') }}</p>
    </section>
  </div>
</template>

<style scoped>
.settings-ai {
  max-width: 560px;
}
.form-section {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}
.form-title {
  margin: 0 0 var(--sp-1);
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-1);
}
.form-hint {
  margin: 0 0 var(--sp-2);
  font-size: var(--fs-xs);
  color: var(--text-3);
  line-height: 1.6;
}
.form-label {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  font-size: var(--fs-xs);
  color: var(--text-2);
  margin-top: var(--sp-1);
}
.key-masked {
  color: var(--text-3);
  font-size: var(--fs-xxs);
}
.form-row {
  display: flex;
  gap: var(--sp-3);
}
.form-col {
  flex: 1;
}
.form-actions {
  display: flex;
  gap: var(--sp-2);
  margin-top: var(--sp-3);
}
.form-note {
  margin: var(--sp-2) 0 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  line-height: 1.6;
}
</style>
