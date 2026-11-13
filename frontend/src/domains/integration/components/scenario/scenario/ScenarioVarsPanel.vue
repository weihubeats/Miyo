<script setup lang="ts">
/**
 * ScenarioVarsPanel：变量链路管理器（从 ScenarioEditor 抽出重构）。
 *
 * 相比旧版（整列变量条目常驻编排区上方）的信息架构调整：
 * - 默认折叠为一行总览：图标 + 标题 + 变量 chip 速览 + 未定义警示；
 * - 展开后提供搜索框 + 分类过滤（全部 / 链路定义 / 外部变量 / 未定义），
 *   长链路场景下能快速定位某个变量；
 * - 条目行展示：变量 chip、状态徽章（链路 / 外部 / 未定义）、来源步骤 +
 *   提取表达式、引用步骤数；点击来源 / 引用定位对应步骤卡（emit locate）；
 * - 运行后展示运行时值（敏感名脱敏 + 眼睛切换，沿用旧脱敏口径）。
 *
 * 数据流不变：链路条目由宿主从启用步骤的真实配置推导（computeVarChain），
 * 本组件只做展示与交互转发，不持有编辑态。
 */
import { computed, ref } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import type { VarChainEntry } from '@/utils/scenarioVars'
import { isSensitiveVarName } from '@/utils/scenarioVars'

const props = defineProps<{
  /** 变量链路条目（宿主 computed 推导）。 */
  chain: VarChainEntry[]
  /** 后端最终变量池（无运行结果为 null）。 */
  runtimeVars: Record<string, string> | null
}>()

const emit = defineEmits<{
  /** 点击来源 / 引用步骤：携带编辑态步骤索引（宿主滚动定位步骤卡）。 */
  (e: 'locate', stepIndex: number): void
}>()

const t = useLocaleStore().t

/** 变量展示形态（模板插值内不能写 {{ 字面量，收敛到函数）。 */
function varTag(n: string): string {
  return `{{${n}}}`
}

// ---------- 折叠 / 展开 ----------
const expanded = ref(false)

// ---------- 搜索 + 分类过滤 ----------
const query = ref('')
type VarFilter = 'all' | 'linked' | 'external' | 'missing'
const filter = ref<VarFilter>('all')

const FILTERS = computed<{ value: VarFilter; label: string }[]>(() => [
  { value: 'all', label: t('scenario.varsFilterAll') },
  { value: 'linked', label: t('scenario.varsFilterLinked') },
  { value: 'external', label: t('scenario.varsFilterExternal') },
  { value: 'missing', label: t('scenario.varsFilterMissing') },
])

const linkedCount = computed(() => props.chain.filter((v) => !v.missing && !v.external).length)
const externalCount = computed(() => props.chain.filter((v) => v.external).length)
const missingCount = computed(() => props.chain.filter((v) => v.missing).length)

/** 搜索 + 过滤后的条目（名称 / 来源步骤名 / 引用步骤名任一命中）。 */
const filtered = computed(() => {
  const q = query.value.trim().toLowerCase()
  return props.chain.filter((v) => {
    if (filter.value === 'linked' && (v.missing || v.external)) return false
    if (filter.value === 'external' && !v.external) return false
    if (filter.value === 'missing' && !v.missing) return false
    if (!q) return true
    if (v.name.toLowerCase().includes(q)) return true
    return (
      v.sources.some((s) => s.stepName.toLowerCase().includes(q)) ||
      v.usages.some((u) => u.stepName.toLowerCase().includes(q))
    )
  })
})

// ---------- 详情展开（单选 accordion） ----------
const expandedVar = ref<string | null>(null)

function toggleVarDetail(name: string): void {
  expandedVar.value = expandedVar.value === name ? null : name
}

/** 运行时值（后端最终变量池）；敏感名默认掩码，眼睛切换。 */
const revealedVars = ref(new Set<string>())

function runtimeValueOf(varName: string): string | null {
  const vars = props.runtimeVars
  if (!vars || !(varName in vars)) return null
  return vars[varName] ?? ''
}

function displayRuntimeValue(varName: string): string {
  const value = runtimeValueOf(varName)
  if (value == null) return ''
  if (isSensitiveVarName(varName) && !revealedVars.value.has(varName)) return '••••••'
  return value
}

function toggleReveal(varName: string): void {
  const next = new Set(revealedVars.value)
  if (next.has(varName)) next.delete(varName)
  else next.add(varName)
  revealedVars.value = next
}

/** 链路条目的一行摘要：来源步骤 + 引用数。 */
function varBrief(v: VarChainEntry): string {
  const usePart = t('scenario.varUseCount', { n: v.usages.length })
  if (v.sources.length) {
    const src = v.sources[0]!
    return `${t('scenario.stepRef', { n: src.stepIndex + 1, name: src.stepName })} · ${usePart}`
  }
  if (v.external) return `${t('scenario.varExternal')} · ${usePart}`
  return usePart
}
</script>

<template>
  <section v-if="chain.length" class="se-vars-panel">
    <!-- 总览行（折叠态）：标题 + chip 速览 + 未定义警示 -->
    <button
      type="button"
      class="se-vars-head"
      :aria-label="expanded ? t('scenario.collapseVars') : t('scenario.expandVars')"
      @click="expanded = !expanded"
    >
      <Icon name="zap" :size="12" class="se-vars-glyph" />
      <span class="se-vars-title">{{ t('scenario.linkVars') }}</span>
      <span class="se-vars-count">{{ t('scenario.varsCount', { n: chain.length }) }}</span>
      <code
        v-for="v in chain.slice(0, 8)"
        :key="v.name"
        class="se-var-chip"
        :class="{ warn: v.missing, ext: v.external }"
        :title="v.missing ? t('scenario.varMissingWarn') : undefined"
      >{{ varTag(v.name) }}</code>
      <span v-if="chain.length > 8" class="se-var-more">+{{ chain.length - 8 }}</span>
      <span v-if="missingCount" class="se-vars-warn">
        <Icon name="alert-triangle" :size="11" />
        {{ t('scenario.varMissing', { n: missingCount }) }}
      </span>
      <span class="se-vars-caret">
        <Icon :name="expanded ? 'chevrons-down-up' : 'chevrons-up-down'" :size="13" />
      </span>
    </button>

    <!-- 展开体：搜索 + 分类过滤 + 条目列表 -->
    <div v-if="expanded" class="se-vars-body">
      <div class="se-vars-toolbar">
        <div class="se-vars-search">
          <Icon name="search" :size="12" class="se-vars-search-icon" />
          <input
            v-model="query"
            class="se-vars-search-input"
            type="text"
            :placeholder="t('scenario.varsSearchPh')"
            spellcheck="false"
          />
          <button
            v-if="query"
            class="se-vars-search-clear"
            type="button"
            :aria-label="t('common.clearSearch')"
            @click="query = ''"
          >
            <Icon name="x" :size="11" />
          </button>
        </div>
        <div class="se-vars-filters" role="tablist">
          <button
            v-for="f in FILTERS"
            :key="f.value"
            type="button"
            class="se-vars-filter"
            :class="{ active: filter === f.value }"
            @click="filter = f.value"
          >
            {{ f.label }}
            <span v-if="f.value === 'missing' && missingCount" class="se-filter-badge warn">{{ missingCount }}</span>
            <span v-else-if="f.value === 'external'" class="se-filter-badge">{{ externalCount }}</span>
            <span v-else-if="f.value === 'linked'" class="se-filter-badge">{{ linkedCount }}</span>
          </button>
        </div>
      </div>

      <EmptyState v-if="!filtered.length" icon="zap" :title="t('scenario.varsEmpty')" compact />

      <div v-else class="se-vars-list">
        <div
          v-for="v in filtered"
          :key="v.name"
          class="se-var-entry"
          :class="{ missing: v.missing }"
        >
          <button
            type="button"
            class="se-var-entry-head"
            :aria-label="t('scenario.varOpenDetail')"
            @click="toggleVarDetail(v.name)"
          >
            <code class="se-var-chip">{{ varTag(v.name) }}</code>
            <span v-if="v.missing" class="se-var-badge warn">{{ t('scenario.varUndefinedBadge') }}</span>
            <span v-else-if="v.external" class="se-var-badge ext">{{ t('scenario.varExternal') }}</span>
            <span v-else class="se-var-badge ok">{{ t('scenario.varLinked') }}</span>
            <span class="se-var-brief">{{ varBrief(v) }}</span>
            <span class="se-var-usecount">{{ t('scenario.varUseCount', { n: v.usages.length }) }}</span>
            <Icon
              name="chevron-down"
              :size="12"
              class="se-var-caret"
              :class="{ open: expandedVar === v.name }"
            />
          </button>
          <div v-if="expandedVar === v.name" class="se-var-detail">
            <div v-if="v.sources.length" class="se-var-line">
              <span class="se-var-k">{{ t('scenario.varSource') }}</span>
              <button
                v-for="src in v.sources"
                :key="src.stepIndex"
                type="button"
                class="se-var-loc"
                :title="t('scenario.locateStep')"
                @click="emit('locate', src.stepIndex)"
              >
                {{ t('scenario.stepRef', { n: src.stepIndex + 1, name: src.stepName }) }}
                <code class="se-var-path">{{ src.path }}</code>
              </button>
            </div>
            <div v-if="v.usages.length" class="se-var-line">
              <span class="se-var-k">{{ t('scenario.varUsages') }}</span>
              <button
                v-for="u in v.usages"
                :key="u.stepIndex"
                type="button"
                class="se-var-loc"
                :title="t('scenario.locateStep')"
                @click="emit('locate', u.stepIndex)"
              >
                {{ t('scenario.stepRef', { n: u.stepIndex + 1, name: u.stepName }) }}
              </button>
            </div>
            <div class="se-var-line">
              <span class="se-var-k">{{ t('scenario.varRuntime') }}</span>
              <template v-if="runtimeValueOf(v.name) !== null">
                <code
                  class="se-var-value"
                  :class="{ masked: isSensitiveVarName(v.name) && !revealedVars.has(v.name) }"
                >{{ displayRuntimeValue(v.name) }}</code>
                <IconButton
                  v-if="isSensitiveVarName(v.name)"
                  name="eye"
                  :size="12"
                  :label="t('scenario.varToggleValue')"
                  :title="t('scenario.varToggleValue')"
                  @click="toggleReveal(v.name)"
                />
              </template>
              <span v-else class="se-var-none">{{ t('scenario.varRuntimeEmpty') }}</span>
            </div>
            <p v-if="v.missing" class="se-var-missing-hint">
              <Icon name="alert-triangle" :size="11" />
              {{ t('scenario.varMissingWarn') }}
            </p>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.se-vars-panel {
  flex: none;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-panel);
}

/* ---- 总览行 ---- */
.se-vars-head {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  width: 100%;
  border: none;
  background: transparent;
  padding: 7px 12px;
  cursor: pointer;
  text-align: left;
  color: var(--text-2);
  border-radius: var(--radius-lg);
}

.se-vars-head:hover {
  background: var(--bg-hover);
}

.se-vars-glyph {
  color: var(--accent);
  flex: none;
}

.se-vars-title {
  flex: none;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-2);
}

.se-vars-count {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  padding: 1px 7px;
  border-radius: 999px;
  background: var(--bg-hover);
}

.se-vars-warn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-xxs);
  color: var(--warning);
  margin-left: auto;
}

.se-vars-caret {
  display: inline-flex;
  margin-left: auto;
  color: var(--text-3);
}

.se-vars-warn + .se-vars-caret {
  margin-left: 0;
}

/* ---- 展开体 ---- */
.se-vars-body {
  border-top: 1px solid var(--border);
  padding: 8px 12px 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.se-vars-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

/* 搜索框：与工作区侧栏同语言的凹陷质感 */
.se-vars-search {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 26px;
  width: 200px;
  padding: 0 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-code);
}

.se-vars-search:focus-within {
  border-color: var(--accent);
}

.se-vars-search-icon {
  flex: none;
  color: var(--text-3);
  opacity: 0.7;
}

.se-vars-search-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  font-size: var(--fs-xs);
  color: var(--text-1);
}

.se-vars-search-input::placeholder {
  color: var(--text-3);
}

.se-vars-search-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  width: 16px;
  height: 16px;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-3);
  cursor: pointer;
  padding: 0;
}

.se-vars-search-clear:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

/* 分类过滤：分段胶囊 */
.se-vars-filters {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 2px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-card);
}

.se-vars-filter {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: none;
  background: transparent;
  color: var(--text-3);
  font-size: var(--fs-xxs);
  padding: 3px 8px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  white-space: nowrap;
}

.se-vars-filter:hover {
  color: var(--text-1);
}

.se-vars-filter.active {
  background: var(--bg-active);
  color: var(--text-1);
}

.se-filter-badge {
  font-size: 10px;
  padding: 0 4px;
  min-width: 14px;
  text-align: center;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-3);
}

.se-filter-badge.warn {
  background: var(--warning-tint);
  color: var(--warning);
}

/* ---- 条目列表 ---- */
.se-vars-list {
  display: flex;
  flex-direction: column;
}

.se-var-entry + .se-var-entry {
  border-top: 1px dashed var(--border);
}

.se-var-entry-head {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  border: none;
  background: transparent;
  padding: 5px 2px;
  cursor: pointer;
  text-align: left;
  color: var(--text-2);
  border-radius: var(--radius-sm);
}

.se-var-entry-head:hover {
  background: var(--bg-hover);
}

.se-var-entry.missing > .se-var-entry-head > .se-var-chip {
  border-color: var(--warning);
  color: var(--warning);
}

.se-var-badge {
  flex: none;
  font-size: var(--fs-xxs);
  padding: 1px 7px;
  border-radius: 999px;
  border: 1px solid var(--border);
  color: var(--text-3);
}

.se-var-badge.ok {
  background: var(--success-tint);
  border-color: transparent;
  color: var(--success);
}

.se-var-badge.ext {
  background: var(--bg-hover);
  border-color: transparent;
  color: var(--text-3);
}

.se-var-badge.warn {
  background: var(--warning-tint);
  border-color: transparent;
  color: var(--warning);
}

.se-var-brief {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.se-var-usecount {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  white-space: nowrap;
}

.se-var-caret {
  flex: none;
  color: var(--text-3);
  transition: transform var(--dur) var(--ease);
}

.se-var-caret.open {
  transform: rotate(180deg);
}

.se-var-detail {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 4px 2px 8px 34px;
}

.se-var-line {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 4px 10px;
  font-size: var(--fs-xxs);
  line-height: 1.6;
}

.se-var-k {
  flex: none;
  width: 52px;
  color: var(--text-3);
}

.se-var-path {
  font-family: var(--font-mono);
  color: var(--tok-key);
  background: var(--bg-code);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0 5px;
}

/* 来源 / 引用步骤：可点击定位 */
.se-var-loc {
  display: inline-flex;
  align-items: baseline;
  gap: 4px;
  border: none;
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-xxs);
  cursor: pointer;
  padding: 1px 4px;
  border-radius: var(--radius-sm);
}

.se-var-loc:hover {
  background: var(--accent-tint);
  color: var(--accent);
}

.se-var-value {
  font-family: var(--font-mono);
  color: var(--text-1);
  background: var(--bg-code);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0 5px;
  max-width: 420px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.se-var-value.masked {
  color: var(--text-3);
  letter-spacing: 2px;
}

.se-var-none {
  color: var(--text-3);
}

.se-var-missing-hint {
  margin: 2px 0 0;
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  color: var(--warning);
}

/* ---- 变量 chip（全局样式，见 style.css 场景编排区） ---- */
</style>
