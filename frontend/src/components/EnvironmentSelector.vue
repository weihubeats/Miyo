<script setup lang="ts">
/**
 * EnvironmentSelector：环境选择器（全局共享，顶栏 / 用例抽屉同源）。
 * - 收起态：状态色点 + 环境名（长名截断），悬停 Tooltip 展示完整 Base URL；
 * - 下拉：搜索框 → 分隔线 → 环境列表（名称 / Base URL 双行 + 选中 ✓）→ 分隔线 → 底部操作栏；
 *   列表项左区 [状态点 | 上下双行文字列]，URL 独占第二行不与名称横向挤压（回归见 spec）；
 * - 选中态 accent 轻底 + accent/30% 边框，✓ 固定最右；hover 渐显复制（/ 编辑）；
 * - 数据与写入同源于 workspace store（environments / activeEnvId / setEnvironment）：
 *   多实例天然一致，无需父级转发；
 * - variant：
 *   · 'group'：融进父级 Pill 组控（无自绘边框 / 底色 / 圆角，几何与焦点环由组控提供）；
 *   · 'solo' ：独立控件（自绘边框 + 内凹底 + radius-md，深浅主题同源变量）；
 * - manage=false：隐藏「编辑环境」「+ 新建环境」「管理环境…」入口与所在弹窗——
 *   用例抽屉遮罩 z-index 120 高于 Modal 的 100，管理弹窗会被盖住；顺带不拉取
 *   EnvironmentManager 大 chunk（按需渲染 + 异步组件）。
 */
import { computed, nextTick, ref } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'
import { useToast } from '../composables/useToast'
import { copyText } from '../utils/clipboard'
import { envBaseUrl, envColorClass } from '../utils/environment'
import { lazyComponent } from '../composables/lazyComponent'
import CustomSelect from './ui/CustomSelect.vue'
import Icon from './ui/Icon.vue'
import Tooltip from './ui/Tooltip.vue'
import type { Environment } from '../types/miyoApi'

withDefaults(
  defineProps<{
    /** 触发器外观：group = 父级 Pill 组控内（顶栏）；solo = 独立控件（抽屉等）。 */
    variant?: 'group' | 'solo'
    /** 环境管理入口（hover 编辑 / 底部新建·管理）：被更高层级遮罩盖住的场景关掉。 */
    manage?: boolean
    /** 浮层最小宽度（触发器较窄时保证选项与 URL 可读）。 */
    popMinWidth?: number
  }>(),
  { variant: 'solo', manage: true, popMinWidth: 320 },
)

// 环境管理大弹窗（1400+ 行）随选择器首次渲染才拉取；manage=false 时连渲染都不发生。
const EnvironmentManager = lazyComponent(() => import('./EnvironmentManager.vue'))

const store = useWorkspaceStore()
const locale = useLocaleStore()
const t = locale.t
const toast = useToast()

const selectRef = ref<InstanceType<typeof CustomSelect> | null>(null)
const searchInputEl = ref<HTMLInputElement | null>(null)
const showManager = ref(false)
/** 环境管理弹窗定位：打开时聚焦的环境 id（null=跟随当前激活环境）。 */
const managerEnvId = ref<string | null>(null)
/** 打开管理弹窗时是否立即新建一条环境（底部「+ 新建环境」）。 */
const managerCreate = ref(false)
/** 下拉菜单是否打开：打开时禁用触发器上的悬浮提示，避免与菜单重叠。 */
const menuOpen = ref(false)
/** 下拉顶部搜索关键词（按环境名 / Base URL 过滤）。 */
const searchQuery = ref('')

const activeEnv = computed(
  () => store.environments.find((e) => e.id === store.activeEnvId) ?? null,
)

const options = computed(() => [
  { value: '', label: t('envbar.noEnv') },
  ...store.environments.map((env) => ({ value: env.id, label: env.name })),
])

/** 过滤后的选项：有关键词时剔除「无环境」，按名称 / Base URL 不分大小写匹配。 */
const filteredOptions = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return options.value
  return options.value.filter((o) => {
    if (o.value === '') return false
    const url = envBaseUrl(envByValue(o.value)).toLowerCase()
    return o.label.toLowerCase().includes(q) || url.includes(q)
  })
})

/** 搜索无结果（有关键词且过滤列表为空）。 */
const noMatch = computed(
  () => !!searchQuery.value.trim() && filteredOptions.value.length === 0,
)

/** 悬停 tooltip：完整 Base URL（无环境 / 未配置时不显示）。 */
const tooltipContent = computed(() => (activeEnv.value ? envBaseUrl(activeEnv.value) : ''))

const displayName = computed(() => activeEnv.value?.name ?? t('envbar.noEnv'))
const displayColorClass = computed(() => envColorClass(activeEnv.value?.name ?? ''))

function envByValue(value: string | number): Environment | null | undefined {
  if (value === '' || value == null) return null
  return store.environments.find((e) => e.id === String(value))
}

/** 选项对应解析后的 Base URL（无环境 / 未配置为空串）。 */
function urlOf(value: string | number): string {
  return envBaseUrl(envByValue(value))
}

function onChange(value: string | number): void {
  void store.setEnvironment(value === '' ? null : String(value))
}

/** 下拉打开：清空上次搜索并聚焦搜索框。 */
async function onMenuOpen(): Promise<void> {
  menuOpen.value = true
  searchQuery.value = ''
  await nextTick()
  searchInputEl.value?.focus()
}

function onMenuClose(): void {
  menuOpen.value = false
  searchQuery.value = ''
}

/** 搜索框 Enter：选中首个匹配项并收起。 */
function onSearchEnter(): void {
  if (!searchQuery.value.trim()) return
  const first = filteredOptions.value[0]
  if (!first) return
  onChange(first.value)
  selectRef.value?.close()
}

/** 搜索框 Esc：有关键词先清空，再按一次收起整个下拉。 */
function onSearchEsc(): void {
  if (searchQuery.value) {
    searchQuery.value = ''
    return
  }
  selectRef.value?.close()
}

/** 快捷复制环境 Base URL（按钮点击已 .stop，不会触发选项选中）。 */
async function copyEnvUrl(value: string | number): Promise<void> {
  const url = urlOf(value)
  if (!url) return
  const ok = await copyText(url)
  if (ok) toast.success(t('editor.baseCopied'))
  else toast.error(t('response.copyFail'))
}

/** 打开环境管理：先收起下拉，再带定位（编辑指定环境 / 新建 / 全局入口）。 */
function openManager(envId: string | null = null, create = false): void {
  selectRef.value?.close()
  managerEnvId.value = envId
  managerCreate.value = create
  showManager.value = true
}

function onManagerOpenChange(open: boolean): void {
  showManager.value = open
  if (!open) {
    managerEnvId.value = null
    managerCreate.value = false
  }
}

// 外部入口（顶栏环境速览的「管理环境」）：管理弹窗为选择器自持，避免两处各挂一份实例。
defineExpose({ openManager })
</script>

<template>
  <div class="env-sel" :class="`env-sel--${variant}`">
    <Tooltip :content="tooltipContent" :disabled="menuOpen" placement="bottom">
      <CustomSelect
        ref="selectRef"
        class="env-sel-select"
        pop-class="env-pop"
        :model-value="store.activeEnvId ?? ''"
        :options="filteredOptions"
        :placeholder="t('envbar.placeholder')"
        size="md"
        :pop-min-width="popMinWidth"
        @change="onChange"
        @open="onMenuOpen"
        @close="onMenuClose"
      >
        <template #display>
          <span class="edot" :class="`ed-${displayColorClass}`"></span>
          <span class="env-display-name">{{ displayName }}</span>
        </template>
        <template #search>
          <div class="es-search">
            <div class="es-search-row">
              <Icon class="es-search-icon" name="search" :size="13" />
              <input
                ref="searchInputEl"
                v-model="searchQuery"
                type="text"
                class="es-search-input"
                :placeholder="t('envbar.search')"
                @keydown.enter.prevent="onSearchEnter"
                @keydown.esc.stop="onSearchEsc"
              />
            </div>
            <div v-if="noMatch" class="es-search-empty">{{ t('envbar.noMatch') }}</div>
          </div>
        </template>
        <template #option="{ option }">
          <div class="env-opt-left">
            <span class="env-dot" :class="`ed-${envColorClass(option.label)}`"></span>
            <div class="env-opt-text">
              <span class="env-opt-name" :title="option.label">{{ option.label }}</span>
              <span v-if="urlOf(option.value)" class="env-opt-url" :title="urlOf(option.value)">
                {{ urlOf(option.value) }}
              </span>
            </div>
          </div>
        </template>
        <template #actions="{ option }">
          <span v-if="option.value !== ''" class="es-actions">
            <button
              v-if="urlOf(option.value)"
              type="button"
              class="es-act"
              :title="t('envbar.copyUrl')"
              :aria-label="t('envbar.copyUrl')"
              @click.stop="copyEnvUrl(option.value)"
            >
              <Icon name="copy" :size="12" />
            </button>
            <button
              v-if="manage"
              type="button"
              class="es-act"
              :title="t('envbar.editEnv')"
              :aria-label="t('envbar.editEnv')"
              @click.stop="openManager(String(option.value))"
            >
              <Icon name="pencil" :size="12" />
            </button>
          </span>
        </template>
        <template v-if="manage" #footer>
          <div class="es-pop-footer">
            <button type="button" class="es-foot-btn" @click.stop="openManager(null, true)">
              <Icon name="plus" :size="12" />
              <span>{{ t('envmgr.addEnv') }}</span>
            </button>
            <button type="button" class="es-foot-btn" @click.stop="openManager()">
              <Icon name="settings" :size="12" />
              <span>{{ t('envbar.manage') }}</span>
            </button>
          </div>
        </template>
      </CustomSelect>
    </Tooltip>
    <EnvironmentManager
      v-if="manage"
      :open="showManager"
      :initial-env-id="managerEnvId"
      :create-new="managerCreate"
      @update:open="onManagerOpenChange"
    />
  </div>
</template>

<style scoped>
/* ---- 触发器几何：两种 variant 统一 32px（size=md → --h-md）/ 12px 中黑字 ----
   注：字号 / 内距靠本组件规则压过 CustomSelect 的 size 档（同特异度比顺序不安全） */
.env-sel {
  display: inline-block;
}

/* 宽度链：父级（顶栏）给外层定宽时一路透传到 .cs-trigger（100%），
   不设宽度则各层按内容收缩（抽屉里自然宽） */
.env-sel :deep(.tt-trigger) {
  width: 100%;
}
.env-sel :deep(.cs) {
  width: 100%;
}

.env-sel :deep(.cs-trigger) {
  gap: 8px;
  font-family: var(--font-ui);
  font-size: 12px;
  font-weight: 500;
  border-radius: var(--radius-md);
}

/* ---- group：融进父级 Pill 组控（.eb-group 提供边框 / 底色 / 焦点环） ---- */
.env-sel--group :deep(.cs-trigger) {
  border: none;
  background: transparent;
  border-radius: 0;
}
.env-sel--group :deep(.cs-trigger:hover:not(:disabled)) {
  background: var(--bg-hover);
}
/* 下拉箭头弱化：更小、更低透明度（点击区仍是整行） */
.env-sel--group :deep(.cs-caret) {
  opacity: 0.6;
  transform: scale(0.85);
}
/* 打开 / 聚焦态由组控的 :focus-within 统一表达，触发器自身不再叠加 accent 环 */
.env-sel--group :deep(.cs.open .cs-trigger) {
  border: none;
  box-shadow: none;
}
.env-sel--group :deep(.cs-trigger:focus-visible) {
  outline: none;
}

/* ---- solo：独立控件（抽屉等）——背景 / 边框 / hover 与顶栏组控同源变量 ---- */
.env-sel--solo {
  min-width: 130px;
}
.env-sel--solo :deep(.cs-trigger) {
  padding: 0 12px;
  border-color: var(--border);
  background: var(--bg-hover);
}
.env-sel--solo :deep(.cs-trigger:hover:not(:disabled)) {
  border-color: var(--border-strong);
  background: var(--bg-active);
}
.env-sel--solo :deep(.cs-caret) {
  opacity: 0.7;
}

/* ---- 触发区展示：仅 状态点 + 环境名 ---- */
.edot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
  background: currentColor;
  color: var(--text-3);
  box-shadow: 0 0 6px color-mix(in srgb, currentColor 45%, transparent);
}

.env-display-name {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-1);
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ---- 下拉顶部搜索 ----
   低调 inset 灰底（text-1 6% 双主题自适应），聚焦不换底只换边——无亮度跳变 */
.es-search-row {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 4px 8px;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--text-1) 6%, transparent);
  transition: border-color var(--dur) var(--ease);
}
.es-search-row:focus-within {
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
}
.es-search-icon {
  flex-shrink: 0;
  width: 13px;
  height: 13px;
  color: var(--text-3);
}
.es-search-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  font-family: var(--font-ui);
  font-size: 12px;
  color: var(--text-1);
}
.es-search-input::placeholder {
  font-size: 12px;
  color: var(--text-3);
}
.es-search-empty {
  padding: 10px 8px 2px;
  text-align: center;
  font-size: var(--fs-xs);
  color: var(--text-3);
}

/* ---- 列表项左区：[状态圆点 | 上下双行文字列] ----
   双行结构（回归锁定见 EnvironmentBar.spec）：
   .env-opt-left > .env-dot + .env-opt-text(.flex-col) > name / url
   —— URL 独占第二行，只在自己行内 truncate，不与名称横向抢宽 */
.env-opt-left {
  order: 1;
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  flex: 1;
}
.env-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
  background: currentColor;
  color: var(--text-3);
  box-shadow: 0 0 6px color-mix(in srgb, currentColor 45%, transparent);
}
.env-opt-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  flex: 1;
}
.env-opt-name {
  font-size: 12px;
  font-weight: 500;
  line-height: 1.3;
  color: var(--text-1);
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
/* URL 次级降噪：muted 灰 + mono，不与名称争视觉焦点 */
.env-opt-url {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  line-height: 1.3;
  color: var(--text-3);
  min-width: 0;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ---- 选项右侧 hover 快捷操作（复制 URL / 编辑）：默认隐身占位，hover/高亮浮现 ---- */
.es-actions {
  order: 2;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 2px;
  opacity: 0;
  transition: opacity var(--dur) var(--ease);
}
.es-act {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  padding: 0;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}
.es-act:hover {
  background: color-mix(in srgb, var(--text-1) 8%, transparent);
  color: var(--text-1);
}

/* ---- 底部固定操作栏：上分隔线 + 两端对齐低调浅灰按钮 ---- */
.es-pop-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  padding: 2px 4px;
}
.es-foot-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 6px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  font-family: var(--font-ui);
  font-size: 12px;
  color: var(--text-3);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.es-foot-btn:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

/* ⚠ popClass 挂在 .cs-pop 自身（同元素双类），必须用 .cs-pop.env-pop，
   写成后代 .env-pop .cs-pop 永不匹配（曾致整套样式静默失效）。 */

/* ---- 浮层容器 ----
   结构：[搜索 → 分隔线] → [列表] → [分隔线 → 底栏]；宽 320、rounded-xl、深投影 */
:global(.cs-pop.env-pop) {
  min-width: 320px;
  max-width: min(360px, calc(100vw - 16px));
  padding: 6px;
  border-radius: var(--radius-xl);
  border-color: var(--border-strong);
  background: var(--bg-elevated);
  box-shadow:
    0 24px 48px -12px rgb(0 0 0 / 0.5),
    0 8px 24px -8px rgb(0 0 0 / 0.4);
}
/* 卡片行：全行 transparent 边框占位（border-box 等高，选中换 accent 边不撑高） */
:global(.cs-pop.env-pop .cs-opt) {
  height: auto;
  min-height: 34px;
  padding: 6px 8px;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  gap: 6px;
  font-family: var(--font-ui);
  white-space: normal;
}
/* 未选中 hover：背景微亮（--bg-hover 深=白6% / 浅=黑6%） */
:global(.cs-pop.env-pop .cs-opt.hl) {
  background: var(--bg-hover);
}
/* 选中态：accent 半透明轻底 + accent/30% 边框，✓ accent */
:global(.cs-pop.env-pop .cs-opt.sel) {
  color: var(--text-1);
  background: var(--accent-tint);
  border-color: color-mix(in srgb, var(--accent) 30%, transparent);
}
/* 选中行再 hover：保 tint 不被 hl 灰底盖掉，边框略加深 */
:global(.cs-pop.env-pop .cs-opt.hl.sel) {
  background: var(--accent-tint);
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
}
:global(.cs-pop.env-pop .cs-opt.sel .cs-opt-check) {
  color: var(--accent);
}
/* 布局序：左区内容(1) → hover 操作(2) → 最右 ✓(3)，状态点在左区行首，互不重叠；
   ✓ 空位也占 16px，未选中行右缘对齐 */
:global(.cs-pop.env-pop .cs-opt-label) {
  order: 1;
  white-space: normal;
  overflow: visible;
  line-height: 1.4;
}
:global(.cs-pop.env-pop .cs-opt-check) {
  order: 3;
  width: 16px;
  height: 16px;
  flex-shrink: 0;
  margin-left: 4px;
}
:global(.cs-pop.env-pop .cs-opt:hover .es-actions),
:global(.cs-pop.env-pop .cs-opt.hl .es-actions) {
  opacity: 1;
}

/* ---- 环境语义色（映射 utils/environment.ts 的 envColorClass）----
   ⚠ 必须排在 .edot / .env-dot 之后：同特异类靠样式表顺序决胜，
   放前面会被 .env-dot 的 text-3 默认色覆盖（列表点全灰踩过坑）。 */
.ed-dev {
  color: var(--success);
}
.ed-test {
  color: var(--info);
}
.ed-staging {
  color: var(--warning);
}
.ed-prod {
  color: var(--orange);
}
.ed-global {
  color: var(--accent);
}
</style>
