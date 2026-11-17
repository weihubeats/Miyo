<script setup lang="ts">
/**
 * PluginConfigForm：插件配置表单（configSchema 驱动，管理中心「配置」页签）。
 *
 * 渲染 PluginConfigSchema（JSON Schema 严格子集）：string / number / boolean /
 * enum 四类字段。required 字段缺失时禁用保存按钮（后端存储层只做键名白名单，
 * 值形状由本表单约束——插件读取时按自身 Schema 解释）。
 */
import { computed } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import type { PluginConfigSchema } from '@/types/generated/PluginConfigSchema'

const props = defineProps<{
  modelValue: Record<string, unknown>
  schema: PluginConfigSchema
  disabled?: boolean
}>()

const emit = defineEmits<{ 'update:modelValue': [value: Record<string, unknown>] }>()

const t = useLocaleStore().t

const fields = computed(() =>
  Object.entries(props.schema.properties ?? {}).map(([key, field]) => ({
    key,
    field,
    required: props.schema.required?.includes(key) ?? false,
  })),
)

/** required 字段是否齐备（缺失禁保存）。 */
const requiredSatisfied = computed(() =>
  fields.value
    .filter((f) => f.required)
    .every((f) => props.modelValue[f.key] !== undefined && props.modelValue[f.key] !== ''),
)

function valueOf(key: string, fallback?: unknown): unknown {
  return props.modelValue[key] ?? fallback ?? null
}

function setField(key: string, value: unknown): void {
  emit('update:modelValue', { ...props.modelValue, [key]: value })
}

function titleOf(key: string, title: string | null | undefined): string {
  return title && title.length > 0 ? title : key
}

function numberModel(key: string): number | null {
  const v = valueOf(key)
  return typeof v === 'number' ? v : null
}
</script>

<template>
  <div class="pcf-root">
    <label v-for="f in fields" :key="f.key" class="pcf-field">
      <span class="pcf-label">
        {{ titleOf(f.key, f.field.title) }}
        <span v-if="f.required" class="pcf-required" aria-hidden="true">*</span>
      </span>

      <!-- boolean：开关 -->
      <span v-if="f.field.type === 'boolean'" class="pcf-control">
        <input
          type="checkbox"
          role="switch"
          class="pcf-switch"
          :checked="valueOf(f.key, f.field.default) === true"
          :disabled="disabled"
          @change="setField(f.key, ($event.target as HTMLInputElement).checked)"
        />
        <span v-if="f.field.description" class="pcf-hint">{{ f.field.description }}</span>
      </span>

      <!-- enum：下拉 -->
      <span v-else-if="f.field.type === 'enum'" class="pcf-control">
        <select
          class="pcf-select"
          :value="String(valueOf(f.key, f.field.default) ?? '')"
          :disabled="disabled"
          @change="setField(f.key, ($event.target as HTMLSelectElement).value)"
        >
          <option v-for="opt in f.field.enum_values ?? []" :key="opt" :value="opt">{{ opt }}</option>
        </select>
        <span v-if="f.field.description" class="pcf-hint">{{ f.field.description }}</span>
      </span>

      <!-- number：数字输入（min/max 约束） -->
      <span v-else-if="f.field.type === 'number'" class="pcf-control">
        <input
          type="number"
          class="pcf-input"
          :value="numberModel(f.key) ?? (f.field.default as number | undefined) ?? null"
          :min="f.field.min ?? undefined"
          :max="f.field.max ?? undefined"
          :disabled="disabled"
          @change="
            setField(
              f.key,
              ($event.target as HTMLInputElement).value === ''
                ? null
                : Number(($event.target as HTMLInputElement).value),
            )
          "
        />
        <span v-if="f.field.description" class="pcf-hint">{{ f.field.description }}</span>
      </span>

      <!-- string：文本 -->
      <span v-else class="pcf-control">
        <input
          type="text"
          class="pcf-input"
          :value="String(valueOf(f.key, f.field.default) ?? '')"
          :disabled="disabled"
          @input="setField(f.key, ($event.target as HTMLInputElement).value)"
        />
        <span v-if="f.field.description" class="pcf-hint">{{ f.field.description }}</span>
      </span>
    </label>
    <p v-if="!requiredSatisfied" class="pcf-missing" role="alert">{{ t('plugins.config.requiredMissing') }}</p>
  </div>
</template>

<style scoped>
.pcf-root {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.pcf-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.pcf-label {
  font-size: var(--fs-xs);
  color: var(--text-2);
  font-weight: 500;
}
.pcf-required {
  color: var(--danger);
}
.pcf-control {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.pcf-input,
.pcf-select {
  height: 30px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-elevated);
  color: var(--text-1);
  font-size: var(--fs-sm);
  font-family: inherit;
}
.pcf-input:focus-visible,
.pcf-select:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
  border-color: var(--accent);
}

.pcf-switch {
  width: 16px;
  height: 16px;
  accent-color: var(--accent);
  cursor: pointer;
}

.pcf-hint {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}

.pcf-missing {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--danger);
}
</style>
