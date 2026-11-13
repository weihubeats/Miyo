<script setup lang="ts">
/**
 * EnvVarTable：环境变量 / 全局变量共用的紧凑变量表（6 列）。
 *
 * 列：变量名 | 远程值 | 本地覆盖值（显式覆盖开关）| 类型（文本 / 密钥）| 启用 | 操作。
 * - 覆盖开关三态（local_override）：开 → 生效值取本地（允许显式空串）；关 → 恒取
 *   远程；未标注（旧数据）显示按「本地非空」推导，改动后写回显式布尔——生效口径
 *   与 utils/environment.ts effectiveVariable / 后端 effective_value 同源；
 * - 类型列：显式 sensitive 标注优先，未标注按键名启发式推导（resolveSensitive）；
 *   密钥值在「隐藏敏感值」开启时打点遮蔽，聚焦该单元格编辑时临时明文；
 * - 工具行：搜索（按 key 过滤显示，保存仍含全部行）、批量导入（key=value /
 *   key: value / JSON 对象）、敏感值遮蔽总开关（v-model 与父级共享持久化）；
 * - 行数据由父级持有：本组件原地改写行对象 / splice 数组，经 change 通知置脏。
 */
import { computed, ref } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import Icon from '../ui/Icon.vue'
import IconButton from '../ui/IconButton.vue'
import CustomSelect from '../ui/CustomSelect.vue'
import Modal from '../ui/Modal.vue'
import Tooltip from '../ui/Tooltip.vue'
import { parseVarBatch, resolveSensitive } from '../../utils/environment'
import { rowKey } from '../../utils/rowKey'
import type { EnvironmentVariable } from '../../types/miyoApi'

const props = defineProps<{
  /** 变量行（父级持有；行对象就地编辑，增删 / 批量合并经事件由父级执行）。 */
  rows: EnvironmentVariable[]
  /** 敏感值遮蔽总开关（父级持久化于 localStorage）。 */
  hideSensitive: boolean
  /** key 输入占位（环境 / 全局变量文案不同）。 */
  keyPlaceholder?: string
}>()

const emit = defineEmits<{
  /** 任意行内容变更（父级据此置脏）。 */
  change: []
  /** 添加空行（父级执行 push + 置脏）。 */
  add: []
  /** 删除行（父级执行 splice + 置脏）。 */
  remove: [index: number]
  /** 批量导入确认（父级执行合并 + 置脏 + 提示）。 */
  applyBatch: [rows: { key: string; remote_value: string }[]]
  'update:hideSensitive': [v: boolean]
}>()

const t = useLocaleStore().t

// ---------- 行内编辑 ----------
function onInput(): void {
  emit('change')
}

function addVariable(): void {
  emit('add')
}

function removeVariable(index: number): void {
  emit('remove', index)
}

// ---------- 本地覆盖开关（三态） ----------
/** 开关显示态：显式字段优先，未标注按「本地非空」推导。 */
function overrideOn(v: EnvironmentVariable): boolean {
  return v.local_override ?? v.local_value.trim() !== ''
}

function setOverride(v: EnvironmentVariable, on: boolean): void {
  v.local_override = on
  emit('change')
}

// ---------- 类型（文本 / 密钥） ----------
function typeOf(v: EnvironmentVariable): 'text' | 'secret' {
  return resolveSensitive(v) ? 'secret' : 'text'
}

/** 类型下拉选项（随语言刷新的常量数组必须包 computed）。 */
const typeOptions = computed(() => [
  { value: 'text', label: t('envmgr.typeText') },
  { value: 'secret', label: t('envmgr.typeSecret') },
])

function setType(v: EnvironmentVariable, value: string | number): void {
  v.sensitive = value === 'secret'
  emit('change')
}

// ---------- 行级显隐（密钥行单独揭示，独立于全局遮蔽总开关） ----------
const revealed = ref(new Set<string>())

function isRevealed(v: EnvironmentVariable): boolean {
  return revealed.value.has(rowKey(v))
}

function toggleReveal(v: EnvironmentVariable): void {
  const k = rowKey(v)
  if (revealed.value.has(k)) revealed.value.delete(k)
  else revealed.value.add(k)
}

// ---------- 敏感值遮蔽（聚焦单元格临时明文） ----------
const focusedCell = ref<string | null>(null)

/** 值列输入形态：总开关关闭 / 非密钥 / 行级揭示 / 聚焦中 → 明文；其余 password。 */
function valueInputType(v: EnvironmentVariable, col: 'remote' | 'local'): 'text' | 'password' {
  if (!props.hideSensitive) return 'text'
  if (!resolveSensitive(v)) return 'text'
  if (revealed.value.has(rowKey(v))) return 'text'
  if (focusedCell.value === `${rowKey(v)}:${col}`) return 'text'
  return 'password'
}

function onCellFocus(v: EnvironmentVariable, col: 'remote' | 'local'): void {
  focusedCell.value = `${rowKey(v)}:${col}`
}

function onCellBlur(): void {
  focusedCell.value = null
}

// ---------- 搜索（仅过滤显示，保存仍含全部行） ----------
const search = ref('')

const visibleVars = computed<{ v: EnvironmentVariable; index: number }[]>(() => {
  const q = search.value.trim().toLowerCase()
  const list: { v: EnvironmentVariable; index: number }[] = []
  props.rows.forEach((v, index) => {
    if (!q || v.key.toLowerCase().includes(q)) list.push({ v, index })
  })
  return list
})

// ---------- 批量导入（key=value / key: value / JSON 对象） ----------
const batchOpen = ref(false)
const batchText = ref('')

function openBatch(): void {
  batchText.value = ''
  batchOpen.value = true
}

const batchParsed = computed(() => parseVarBatch(batchText.value))

/** 预览统计：新增 n 个 / 更新 n 个（命中已有 key 就地覆盖远程值）/ 无效 n 行。 */
const batchStats = computed(() => {
  const keys = new Set(props.rows.map((v) => v.key.trim()))
  let add = 0
  let update = 0
  for (const r of batchParsed.value.rows) {
    if (keys.has(r.key)) update += 1
    else add += 1
  }
  return { add, update, invalid: batchParsed.value.invalid }
})

function applyBatch(): void {
  const { rows } = batchParsed.value
  if (!rows.length) return
  emit('applyBatch', rows)
  batchOpen.value = false
}
</script>

<template>
  <div class="evt">
    <!-- 工具行：搜索 + 批量导入 + 敏感值遮蔽总开关 -->
    <div class="evt-toolbar">
      <div class="evt-search">
        <Icon name="search" :size="12" />
        <input v-model="search" class="evt-search-input" :placeholder="t('envmgr.searchPh')" spellcheck="false" />
      </div>
      <button class="evt-tool-btn" type="button" :title="t('envmgr.batchImportTitle')" @click="openBatch">
        <Icon name="file-plus" :size="12" />
        {{ t('envmgr.batchImport') }}
      </button>
      <button
        class="evt-tool-btn"
        :class="{ on: hideSensitive }"
        type="button"
        :title="t('envmgr.maskToggleTip')"
        @click="emit('update:hideSensitive', !hideSensitive)"
      >
        <Icon :name="hideSensitive ? 'eye-off' : 'eye'" :size="12" />
        {{ t('envmgr.maskSensitive') }}
      </button>
    </div>

    <div class="em-table">
      <!-- grid 列宽 17/25/28/13/8/9（fr 保持配比且不被 gap 挤压） -->
      <div class="em-th evt-head">
        <span class="evt-c-key">{{ t('envmgr.colKey') }}</span>
        <span class="evt-c-remote">{{ t('envmgr.colRemote') }}</span>
        <span class="evt-c-local">{{ t('envmgr.colLocal') }}</span>
        <span class="evt-c-type">{{ t('envmgr.colType') }}</span>
        <span class="evt-c-enabled">{{ t('envmgr.colEnabled') }}</span>
        <span class="evt-c-op"></span>
      </div>
      <div
        v-for="{ v, index } in visibleVars"
        :key="rowKey(v)"
        class="em-tr evt-row"
        :class="{ off: !v.enabled }"
      >
        <input
          v-model="v.key"
          class="miyo-input miyo-input-sm evt-c-key"
          :placeholder="keyPlaceholder ?? t('envmgr.varKeyPh')"
          spellcheck="false"
          @input="onInput"
        />
        <input
          v-model="v.remote_value"
          :type="valueInputType(v, 'remote')"
          class="miyo-input miyo-input-sm evt-c-remote"
          :placeholder="t('envmgr.varRemotePh')"
          spellcheck="false"
          @input="onInput"
          @focus="onCellFocus(v, 'remote')"
          @blur="onCellBlur"
        />
        <span class="evt-c-local" :class="{ disabled: !overrideOn(v) }">
          <!-- 覆盖开关：开 = 生效值取本地（允许显式空串）；关 = 恒用远程值 -->
          <Tooltip :content="t('envmgr.overrideTip')" placement="top">
            <input
              type="checkbox"
              class="evt-ov-check"
              :checked="overrideOn(v)"
              :aria-label="t('envmgr.colOverride')"
              @change="setOverride(v, ($event.target as HTMLInputElement).checked)"
            />
          </Tooltip>
          <input
            v-model="v.local_value"
            :type="valueInputType(v, 'local')"
            class="miyo-input miyo-input-sm evt-local-input"
            :disabled="!overrideOn(v)"
            :placeholder="!overrideOn(v) ? t('envmgr.overrideOffPh') : v.local_value ? t('envmgr.varLocalPh') : t('envmgr.overrideEmptyPh')"
            spellcheck="false"
            @input="onInput"
            @focus="onCellFocus(v, 'local')"
            @blur="onCellBlur"
          />
        </span>
        <CustomSelect
          :model-value="typeOf(v)"
          :options="typeOptions"
          size="sm"
          class="evt-type-select"
          :aria-label="t('envmgr.colType')"
          @update:model-value="setType(v, $event)"
        />
        <span class="evt-c-enabled">
          <input
            v-model="v.enabled"
            type="checkbox"
            class="evt-enabled-check"
            :checked="v.enabled"
            :aria-label="t('envmgr.colEnabled')"
            @change="onInput"
          />
        </span>
        <span class="evt-c-op">
          <IconButton
            v-if="typeOf(v) === 'secret'"
            :name="isRevealed(v) ? 'eye-off' : 'eye'"
            :size="13"
            :title="isRevealed(v) ? t('envmgr.hideValues') : t('envmgr.showValues')"
            @click="toggleReveal(v)"
          />
          <IconButton
            name="trash"
            :size="13"
            tone="danger"
            :title="t('envmgr.deleteVar')"
            @click="removeVariable(index)"
          />
        </span>
      </div>
      <div v-if="search.trim() && !visibleVars.length" class="evt-no-match">
        {{ t('envmgr.searchEmpty') }}
      </div>
      <button class="miyo-btn miyo-btn-sm em-add-var" type="button" @click="addVariable">
        <Icon name="plus" :size="13" /> {{ t('envmgr.addVar') }}
      </button>
    </div>

    <!-- 批量导入 -->
    <Modal
      v-if="batchOpen"
      :open="batchOpen"
      :title="t('envmgr.batchImportTitle')"
      width="480px"
      @update:open="batchOpen = $event"
      @close="batchOpen = false"
    >
      <p class="evt-batch-hint">{{ t('envmgr.batchImportHint') }}</p>
      <textarea
        v-model="batchText"
        class="miyo-input evt-batch-input"
        spellcheck="false"
        :placeholder="'token = abc123\nhost: https://api.example.com'"
      ></textarea>
      <p v-if="batchText.trim() && !batchParsed.rows.length" class="evt-batch-none">
        {{ t('envmgr.batchImportEmpty') }}
      </p>
      <p v-else-if="batchParsed.rows.length" class="evt-batch-stats">
        {{ t('envmgr.batchPreview', { add: batchStats.add, update: batchStats.update }) }}
        <template v-if="batchStats.invalid.length">
          {{ t('envmgr.batchInvalid', { n: batchStats.invalid.length }) }}
        </template>
      </p>
      <template #footer>
        <button class="miyo-btn" type="button" @click="batchOpen = false">{{ t('common.cancel') }}</button>
        <button
          class="miyo-btn miyo-btn-primary"
          type="button"
          :disabled="!batchParsed.rows.length"
          @click="applyBatch"
        >
          {{ t('envmgr.batchImportApply') }}
        </button>
      </template>
    </Modal>
  </div>
</template>

<style scoped>
/* 工具行：搜索占满剩余宽度，右侧两个文字钮 */
/* 占满父级剩余高度：变量表（.em-table）才能撑开内部滚动区。
   父级（EnvWorkspace .em-section-vars）是 flex 列容器，这里必须 flex:1 + min-height:0，
   否则整块按内容高度撑开，变量一多就把 .em-table 的滚动区顶出可视区域——
   表现为「环境变量太多无法上下滚动」（issue #172）。 */
.evt {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex: 1;
  min-height: 0;
}

.evt-toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
}

.evt-search {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  height: var(--h-sm);
  padding: 0 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-code);
  color: var(--text-3);
}

.evt-search:focus-within {
  border-color: color-mix(in srgb, var(--accent) 55%, transparent);
}

.evt-search-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: none;
  font-family: inherit;
  font-size: 12px;
  color: var(--text-1);
}

.evt-tool-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: var(--h-sm);
  padding: 0 8px;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  font-family: inherit;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  cursor: pointer;
  white-space: nowrap;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}

.evt-tool-btn:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

.evt-tool-btn.on {
  color: var(--accent);
}

/* grid 列宽配比 17 / 25 / 28 / 13 / 8 / 9（fr 单位：扣掉 gap 后按比例分配，不溢出） */
.evt-head,
.evt-row {
  display: grid;
  grid-template-columns: 17fr 25fr 28fr 13fr 8fr 9fr;
  gap: 8px;
  align-items: center;
}

.evt-c-key,
.evt-c-remote {
  min-width: 0;
}

.evt-c-remote {
  font-family: var(--font-mono);
}

/* 本地覆盖值单元格：覆盖开关 + 输入框 */
.evt-c-local {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 5px;
}

.evt-c-local.disabled .evt-local-input {
  opacity: 0.55;
}

.evt-ov-check {
  flex-shrink: 0;
  accent-color: var(--accent);
  cursor: pointer;
}

.evt-local-input {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
}

/* 类型列：项目统一 CustomSelect（不用浏览器原生 select 样式） */
.evt-c-type {
  min-width: 0;
}

.evt-type-select {
  width: 100%;
}

/* 启用列：checkbox 居中 */
.evt-c-enabled {
  display: flex;
  justify-content: center;
}

.evt-enabled-check {
  accent-color: var(--accent);
  cursor: pointer;
}

/* 操作列：行级显隐（密钥行）+ 删除，右对齐 */
.evt-c-op {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 2px;
}

.evt-no-match {
  padding: 10px 8px;
  font-size: 12px;
  color: var(--text-3);
  text-align: center;
}

/* 批量导入弹窗 */
.evt-batch-hint {
  margin: 0 0 8px;
  font-size: 12px;
  color: var(--text-2);
}

.evt-batch-input {
  width: 100%;
  min-height: 120px;
  font-family: var(--font-mono);
  font-size: 12px;
  resize: vertical;
}

.evt-batch-none {
  margin: 8px 0 0;
  font-size: 12px;
  color: var(--text-3);
}

.evt-batch-stats {
  margin: 8px 0 0;
  padding: 8px 10px;
  border-radius: var(--radius-sm);
  background: var(--accent-tint);
  color: var(--accent);
  font-size: 12px;
}
</style>
