<script setup lang="ts">
/**
 * GrpcPanel：gRPC 端点的主配置面板（method === 'GRPC' 时的 body 页签）。
 *
 * - 地址存 Endpoint.path（编辑器地址栏），metadata 复用 HeadersPanel；
 * - 服务 / 方法选择器：服务目录来自 proto 编译（免连服务器）或服务端反射，
 *   目录按端点缓存，可强制刷新；proto_ids 为空时用反射；
 * - 请求消息为 protobuf-JSON 文本（JsonCodeMirror 编辑）；
 * - proto 文件区：项目级 .proto 导入 / 列表 / 删除 + 端点引用勾选。
 */
import { computed, ref, watch } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useToast } from '../composables/useToast'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useLocaleStore } from '../stores/locale'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import Popconfirm from './ui/Popconfirm.vue'
import Tooltip from './ui/Tooltip.vue'
import CustomSelect from './ui/CustomSelect.vue'
import JsonEditor from './ui/JsonEditor.vue'
import type { Endpoint, ProtoFile, ServiceCatalog } from '../types/miyoApi'

const props = defineProps<{ draft: Endpoint }>()

const store = useWorkspaceStore()
const toast = useToast()
const api = useMiyoApi()
const locale = useLocaleStore()
const t = locale.t

/** 端点 gRPC 配置（body mode = grpc；方法入口已保证形状，这里兜底缺省）。 */
const spec = computed(() => {
  const body = props.draft.request.body
  if (body.mode !== 'grpc') {
    return { service: '', method: '', message: '{}', use_tls: false, proto_ids: [] as string[] }
  }
  return body.spec
})

function patchSpec(patch: Partial<typeof spec.value>): void {
  const body = props.draft.request.body
  if (body.mode !== 'grpc') return
  Object.assign(body.spec, patch)
}

// ---------- 服务目录（按端点缓存） ----------
const catalog = ref<ServiceCatalog | null>(null)
const catalogLoading = ref(false)
const catalogError = ref<string | null>(null)
/** 目录缓存的端点 id：与当前端点不一致时视为过期（切端点不串目录）。 */
const catalogEndpointId = ref<string | null>(null)

const catalogServiceOptions = computed(() =>
  (catalog.value?.services ?? []).map((s) => ({ value: s.service, label: s.service })),
)

/** 当前选中服务的方法选项（附流式标记与输入类型提示）。 */
const catalogMethodOptions = computed(() => {
  const svc = catalog.value?.services.find((s) => s.service === spec.value.service)
  if (!svc) return []
  return svc.methods.map((m) => ({
    value: m.method,
    label: `${m.method}（${m.input_type} → ${m.output_type}）`,
  }))
})

/** 当前方法的元信息（流式徽标 / 输入类型提示用）。 */
const currentMethodInfo = computed(() => {
  const svc = catalog.value?.services.find((s) => s.service === spec.value.service)
  return svc?.methods.find((m) => m.method === spec.value.method) ?? null
})

async function loadCatalog(forceReload = false): Promise<void> {
  if (!props.draft.path.trim()) {
    toast.warning(t('grpc.addressRequired'))
    return
  }
  catalogLoading.value = true
  catalogError.value = null
  try {
    const result = await api.grpcListServices({
      address: props.draft.path,
      use_tls: spec.value.use_tls,
      proto_ids: spec.value.proto_ids,
      project_id: store.project?.id ?? null,
      environment_id: store.activeEnvId,
      force_reload: forceReload,
    })
    catalog.value = result
    catalogEndpointId.value = props.draft.id
  } catch (err) {
    catalogError.value = err instanceof Error ? err.message : String(err)
  } finally {
    catalogLoading.value = false
  }
}

/** 切端点 / 切 proto 引用后目录过期。 */
watch(
  () => [props.draft.id, spec.value.proto_ids.join(','), spec.value.use_tls] as const,
  () => {
    if (catalogEndpointId.value !== props.draft.id) {
      catalog.value = null
      catalogEndpointId.value = null
    }
  },
)

function onServicePick(value: string): void {
  patchSpec({ service: value, method: '' })
}

// ---------- proto 文件管理（项目级） ----------
const protoFiles = ref<ProtoFile[]>([])
const protoLoading = ref(false)

async function loadProtoFiles(): Promise<void> {
  if (!store.project) return
  protoLoading.value = true
  try {
    protoFiles.value = await api.grpcListProtoFiles(store.project.id)
  } catch (err) {
    toast.error(t('grpc.protoLoadFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    protoLoading.value = false
  }
}

void loadProtoFiles()
watch(
  () => store.project?.id,
  () => void loadProtoFiles(),
)

const protoFileInput = ref<HTMLInputElement | null>(null)
function pickProtoFiles(): void {
  protoFileInput.value?.click()
}

async function onProtoFilesChosen(event: Event): Promise<void> {
  const input = event.target as HTMLInputElement
  const files = Array.from(input.files ?? [])
  input.value = ''
  if (files.length === 0 || !store.project) return
  try {
    const payloads = await Promise.all(
      files.map(async (f) => ({
        id: protoFiles.value.find((p) => p.name === f.name)?.id ?? null,
        name: f.name,
        content: await f.text(),
      })),
    )
    protoFiles.value = await api.grpcSaveProtoFiles(store.project.id, payloads)
    toast.success(t('grpc.protoImported', { n: String(payloads.length) }))
  } catch (err) {
    toast.error(t('grpc.protoSaveFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  }
}

async function removeProtoFile(file: ProtoFile): Promise<void> {
  try {
    await api.grpcDeleteProtoFile(file.id)
    protoFiles.value = protoFiles.value.filter((p) => p.id !== file.id)
    patchSpec({ proto_ids: spec.value.proto_ids.filter((id) => id !== file.id) })
  } catch (err) {
    toast.error(t('grpc.protoDeleteFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  }
}

function toggleProtoRef(file: ProtoFile): void {
  const ids = new Set(spec.value.proto_ids)
  if (ids.has(file.id)) ids.delete(file.id)
  else ids.add(file.id)
  patchSpec({ proto_ids: [...ids] })
}


const streamBadge = computed(() => {
  const m = currentMethodInfo.value
  if (!m) return null
  if (m.server_streaming) return t('grpc.streamServer')
  if (m.client_streaming) return t('grpc.streamClient')
  return null
})
</script>

<template>
  <div class="grpc-panel">
    <!-- 服务 / 方法 -->
    <div class="grpc-row">
      <div class="grpc-field service-field">
        <label class="grpc-label">{{ t('grpc.service') }}</label>
        <div class="service-picker">
          <input
            class="miyo-input"
            :value="spec.service"
            spellcheck="false"
            :placeholder="t('grpc.servicePh')"
            @input="patchSpec({ service: ($event.target as HTMLInputElement).value })"
          />
          <CustomSelect
            v-if="catalogServiceOptions.length"
            class="service-catalog-select"
            :model-value="spec.service"
            :options="catalogServiceOptions"
            :placeholder="t('grpc.pickService')"
            @update:model-value="onServicePick(String($event))"
          >
            <template #display="{ label }">
              <span class="catalog-display">{{ label }}</span>
            </template>
          </CustomSelect>
        </div>
      </div>
      <div class="grpc-field method-field">
        <label class="grpc-label">{{ t('grpc.method') }}</label>
        <div class="method-picker">
          <input
            class="miyo-input"
            :value="spec.method"
            spellcheck="false"
            :placeholder="t('grpc.methodPh')"
            @input="patchSpec({ method: ($event.target as HTMLInputElement).value })"
          />
          <CustomSelect
            v-if="catalogMethodOptions.length"
            class="method-catalog-select"
            :model-value="spec.method"
            :options="catalogMethodOptions"
            :placeholder="t('grpc.pickMethod')"
            @update:model-value="patchSpec({ method: String($event) })"
          >
            <template #display="{ label }">
              <span class="catalog-display">{{ label }}</span>
            </template>
          </CustomSelect>
        </div>
      </div>
      <label class="grpc-tls" :title="t('grpc.tlsHint')">
        <input
          :checked="spec.use_tls"
          type="checkbox"
          @change="patchSpec({ use_tls: ($event.target as HTMLInputElement).checked })"
        />
        TLS
      </label>
      <Tooltip :content="t('grpc.reloadCatalog')" placement="top">
        <IconButton name="refresh" :size="14" :disabled="catalogLoading" @click="void loadCatalog(true)" />
      </Tooltip>
    </div>

    <div class="grpc-catalog-row">
      <button class="miyo-btn miyo-btn-sm" type="button" :disabled="catalogLoading" @click="void loadCatalog()">
        <Icon name="list" :size="13" />
        {{ catalog ? t('grpc.reloadCatalogShort') : t('grpc.loadCatalog') }}
      </button>
      <span v-if="catalog" class="catalog-source">
        {{ catalog.source === 'reflection' ? t('grpc.sourceReflection') : t('grpc.sourceProto') }}
        · {{ t('grpc.serviceCount', { n: catalog.services.length }) }}
      </span>
      <span v-if="streamBadge" class="stream-badge">{{ streamBadge }}</span>
      <span v-if="catalogError" class="catalog-error">{{ catalogError }}</span>
    </div>

    <!-- 请求消息 -->
    <div class="grpc-message">
      <div class="grpc-message-head">
        <label class="grpc-label">{{ t('grpc.message') }}</label>
        <span v-if="currentMethodInfo" class="grpc-io">{{ currentMethodInfo.input_type }}</span>
      </div>
      <div class="grpc-message-editor">
        <JsonEditor
          :model-value="spec.message"
          :placeholder="t('grpc.messagePh')"
          :min-height="140"
          @update:model-value="patchSpec({ message: $event })"
        />
      </div>
    </div>

    <!-- proto 文件区（项目级） -->
    <div class="grpc-proto">
      <div class="grpc-proto-head">
        <label class="grpc-label">{{ t('grpc.protoFiles') }}</label>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="pickProtoFiles">
          <Icon name="upload" :size="13" /> {{ t('grpc.protoImport') }}
        </button>
        <input
          ref="protoFileInput"
          class="proto-file-input"
          type="file"
          multiple
          accept=".proto"
          @change="onProtoFilesChosen"
        />
      </div>
      <p class="grpc-proto-hint">{{ t('grpc.protoHint') }}</p>
      <div v-if="protoFiles.length" class="proto-list">
        <div v-for="file in protoFiles" :key="file.id" class="proto-row">
          <label class="proto-check">
            <input
              type="checkbox"
              :checked="spec.proto_ids.includes(file.id)"
              @change="toggleProtoRef(file)"
            />
            <span class="proto-name">{{ file.name }}</span>
          </label>
          <Popconfirm :title="t('grpc.protoDeleteConfirm', { name: file.name })" @confirm="void removeProtoFile(file)">
            <IconButton name="trash" :size="13" tone="danger" :title="t('grpc.protoDelete')" />
          </Popconfirm>
        </div>
      </div>
      <p v-else-if="!protoLoading" class="proto-empty">{{ t('grpc.protoEmpty') }}</p>
    </div>
  </div>
</template>

<style scoped>
.grpc-panel {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 4px 2px 8px;
}
.grpc-row {
  display: flex;
  align-items: flex-end;
  gap: 8px;
  flex-wrap: wrap;
}
.grpc-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.service-field {
  flex: 1.4;
  min-width: 220px;
}
.method-field {
  flex: 1;
  min-width: 180px;
}
.grpc-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  letter-spacing: 0.02em;
}
.service-picker,
.method-picker {
  display: flex;
  gap: 6px;
  align-items: center;
}
.service-picker .miyo-input,
.method-picker .miyo-input {
  flex: 1;
  min-width: 0;
}
.service-catalog-select,
.method-catalog-select {
  width: 26px;
  flex: none;
}
.catalog-display {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.grpc-tls {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 12px;
  color: var(--text-2);
  padding-bottom: 6px;
  cursor: pointer;
  user-select: none;
}
.grpc-catalog-row {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  min-height: 24px;
}
.catalog-source {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.catalog-error {
  font-size: var(--fs-xxs);
  color: var(--danger);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 60%;
}
.stream-badge {
  font-size: var(--fs-xxs);
  padding: 1px 7px;
  border-radius: 999px;
  color: var(--grpc);
  background: color-mix(in srgb, var(--grpc) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--grpc) 25%, transparent);
}
.grpc-message {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.grpc-message-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.grpc-io {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  font-family: var(--font-mono, monospace);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.grpc-message-editor {
  position: relative;
  border: 1px solid var(--border-editor);
  border-radius: var(--radius-md);
  background: var(--bg-code);
  overflow: hidden;
}
.grpc-proto {
  display: flex;
  flex-direction: column;
  gap: 6px;
  border-top: 1px solid var(--border);
  padding-top: 10px;
}
.grpc-proto-head {
  display: flex;
  align-items: center;
  gap: 10px;
}
.proto-file-input {
  display: none;
}
.grpc-proto-hint {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.proto-list {
  display: flex;
  flex-direction: column;
}
.proto-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 4px 6px;
  border-radius: var(--radius-sm);
}
.proto-row:hover {
  background: var(--bg-hover);
}
.proto-check {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  cursor: pointer;
}
.proto-name {
  font-size: 12px;
  color: var(--text-1);
  font-family: var(--font-mono, monospace);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.proto-empty {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  padding: 2px 6px;
}
</style>
