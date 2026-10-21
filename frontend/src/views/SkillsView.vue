<script setup lang="ts">
/**
 * SkillsView：Agent Skill 管理页（与仪表板同构）。
 *
 * 布局：顶栏 + 左侧导航（DashboardNav）+ 主区内左右分栏：
 * - 左：skill 列表（作用域切换 / 新建 / 打开真源 / 刷新）；
 * - 右：选中 skill 的详情与编辑器——**同一路由内切换**，点列表不跳页面。
 *
 * 数据全部来自 fox-skill 的文件系统扫描；视图不维护镜像状态。
 */
import { computed, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { openPath } from '@tauri-apps/plugin-opener'
import Icon from '../components/ui/Icon.vue'
import IconButton from '../components/ui/IconButton.vue'
import SettingsDialog from '../components/SettingsDialog.vue'
import HeaderUpdateTag from '../components/HeaderUpdateTag.vue'
import DashboardNav from '../components/projectlist/DashboardNav.vue'
import JsonCodeMirror from '../components/JsonCodeMirror.vue'
import ToolChip from '../components/skills/ToolChip.vue'
import DeleteConfirmModal from '../components/DeleteConfirmModal.vue'
import { useToast } from '../composables/useToast'
import { useWindowDrag } from '../composables/useWindowDrag'
import { useSkillsStore } from '../stores/skills'
import { useLocaleStore } from '../stores/locale'
import logo from '../assets/rustfox-logo.png'
import type { SkillDoc, SkillSummary, SkillTargetState } from '../types/foxApi'

const router = useRouter()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t
const store = useSkillsStore()

const busy = ref('')

/** 右侧详情面板：当前打开的 skill（点列表选中，不发生路由跳转）。 */
const selectedName = ref('')
const doc = ref<SkillDoc | null>(null)
const loadingDoc = ref(false)

/** 面板内编辑态：默认只读预览，点「编辑」才切表单；无 doc 时为创建态。 */
const editing = ref(false)
const draftName = ref('')
const draftDescription = ref('')
const draftBody = ref('')
const saving = ref(false)
const showSettings = ref(false)

/** 删除确认状态。 */
const deleteOpen = ref(false)
const deletingName = ref('')

const hasSkills = computed(() => store.skills.length > 0)
const warningCount = computed(() =>
  store.skills.reduce((sum, skill) => sum + skill.warnings.length, 0),
)

/** 列表筛选：按名称 / 描述搜索，只看有问题的 skill，按工具过滤。 */
const query = ref('')
const onlyIssues = ref(false)

/** 工具 ID 集合（勾选即显示包含该工具的 skill）。 */
const toolFilter = ref<string[]>([])

const filteredSkills = computed(() => {
  const needle = query.value.trim().toLowerCase()
  return store.skills.filter((skill) => {
    if (needle) {
      const haystack = `${skill.name}\n${skill.description}`.toLowerCase()
      if (!haystack.includes(needle)) return false
    }
    if (onlyIssues.value) {
      // 「问题」= 扫描告警、分叉副本、失效链接；仅仅是「未启用」不算问题。
      const hasIssue =
        skill.warnings.length > 0 ||
        skill.states.some(
          (target) => target.state === 'drifted' || target.state === 'broken',
        )
      if (!hasIssue) return false
    }
    if (toolFilter.value.length) {
      const enabled = skill.states.filter((target) => target.state === 'linked').map((target) => target.toolId)
      if (!toolFilter.value.every((id) => enabled.includes(id))) return false
    }
    return true
  })
})

/** 生效中的筛选数量（不含只看问题，它单独显示为可关闭的 pill）。 */
const activeFilterCount = computed(
  () => (query.value.trim() ? 1 : 0) + toolFilter.value.length,
)

/** 工具筛选默认收起，不占列表纵向空间。 */
const toolsOpen = ref(false)

/** 可过滤的工具：取已出现过的工具，顺序沿用索引里的工具表。 */
const filterTools = computed(() => store.tools)

function toggleToolFilter(toolId: string): void {
  const at = toolFilter.value.indexOf(toolId)
  if (at >= 0) toolFilter.value.splice(at, 1)
  else toolFilter.value.push(toolId)
}

function clearFilters(): void {
  query.value = ''
  onlyIssues.value = false
  toolFilter.value = []
  toolsOpen.value = false
}

/** 筛选结果变化时，选中项不在结果里就自动挑第一条。 */
watch(filteredSkills, (list) => {
  if (editing.value) return
  if (selectedName.value && list.some((skill) => skill.name === selectedName.value)) return
  const first = list[0]
  if (first) void select(first)
  else {
    selectedName.value = ''
    doc.value = null
  }
})

/**
 * 「使用方式」：这类 skill 把调用形态写在 description 里
 * （`别名：/x`、`触发条件：`、`输入形式举例：` 或带引号的命令示例）。
 * 这里只做零风险提取，不做 Markdown 渲染；找不到就展示完整描述。
 */
const USAGE_HEAD_RE =
  /^(别名|触发条件|输入形式举例|输入形式|使用方式|用法|aliases?|triggers?|usage)\s*[:：]\s*(.*)$/i
/** 命令候选：优先反引号，其次中英文引号。 */
const USAGE_SPAN_RE = /`([^`\n]{1,160})`|"([^"\n]{2,160})"|\u201c([^\u201d\n]{2,160})\u201d/g
const USAGE_QUOTE_SYNTAX_RE = /(command|cmd|command\s+line|命令)/i

interface UsageHeading {
  head: string
  lines: string[]
  spans: string[]
}

/** 已知标志词 → 本地化标题（未知词原样显示，避免翻译不了的自造词）。 */
const USAGE_LABEL_KEY: Record<string, string> = {
  别名: 'skills.usageAlias',
  aliases: 'skills.usageAlias',
  alias: 'skills.usageAlias',
  触发条件: 'skills.usageTrigger',
  triggers: 'skills.usageTrigger',
  trigger: 'skills.usageTrigger',
  输入形式: 'skills.usageInput',
  输入形式举例: 'skills.usageInput',
}

/** 详情面板展示的用法区块（无可用结构时为空数组）。 */
const usage = computed<UsageHeading[]>(() => {
  const raw = doc.value?.frontmatter.description ?? ''
  if (!raw.trim()) return []

  const blocks: UsageHeading[] = []
  let current: UsageHeading | null = null

  for (const rawLine of raw.split('\n')) {
    const line = rawLine.trim()
    const match = USAGE_HEAD_RE.exec(line)
    if (match) {
      current = { head: match[1], lines: [], spans: [] }
      blocks.push(current)
      if (match[2]) current.lines.push(match[2])
      continue
    }
    if (!current) continue
    // 缩进的 yaml 片段（harness_lifecycle: …）不算用法，遇到就收尾。
    if (/^[A-Za-z_][A-Za-z0-9_]*\s*:/.test(line)) {
      current = null
      continue
    }
    current.lines.push(line.replace(/^[-*]\s*/, ''))
  }

  for (const block of blocks) {
    const spans: string[] = []
    block.lines = block.lines.filter((line) => {
      for (const found of line.matchAll(USAGE_SPAN_RE)) {
        const value = (found[1] ?? found[2] ?? found[3] ?? '').trim()
        // 反引号一定是指命令；引号包裹的只在行里明确写了「命令」时才收，
        // 否则普通引号短语会被误判成命令。
        const isBacktick = found[1] !== undefined
        if (value && (isBacktick || USAGE_QUOTE_SYNTAX_RE.test(line))) spans.push(value)
      }
      // 整行只剩命令时由 spans 表达，避免重复展示。
      return line.replace(USAGE_SPAN_RE, '').replace(/[（()）、,，]/g, '').trim().length > 0
    })
    block.spans = spans
  }

  return blocks.filter((block) => block.lines.length || block.spans.length)
})

/** 与后端 `validate_name` 保持一致的宽松规则（ASCII 字母数字 + . _ -）。 */
const NAME_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/

/**
 * 未启用徽章不进常规流：它们挂在弹出的 `.tools-hidden` 层里。
 * 直接 display 切换会把条目撑高，进而拖动整列（甚至出现滚动条）跳动。
 */
function visibleStates(skill: SkillSummary): SkillTargetState[] {
  return skill.states.filter((state) => state.state !== 'missing')
}

function hiddenStates(skill: SkillSummary): SkillTargetState[] {
  return skill.states.filter((state) => state.state === 'missing')
}

/** 列表项左侧状态点：全绿=ok，有分叉/失效=异常，全未启用=off。 */
type ItemTone = 'ok' | 'warn' | 'off'

const TONE_RANK: Record<ItemTone, number> = { warn: 2, ok: 1, off: 0 }

function toneOf(state: SkillTargetState['state']): ItemTone {
  if (state === 'drifted' || state === 'broken') return 'warn'
  if (state === 'linked') return 'ok'
  return 'off'
}

function itemTone(skill: SkillSummary): ItemTone {
  return skill.states.reduce<ItemTone>(
    (worst, target) => (TONE_RANK[toneOf(target.state)] > TONE_RANK[worst] ? toneOf(target.state) : worst),
    'off',
  )
}

function chipBusy(skill: SkillSummary, target: SkillTargetState): boolean {
  return busy.value === `${skill.name}:${target.toolId}`
}

/** 详情页路径框的一键复制；失败时静默（不打断阅读）。 */
const copiedPath = ref(false)

async function copyPath(value: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(value)
    copiedPath.value = true
    window.setTimeout(() => (copiedPath.value = false), 1200)
  } catch {
    // 剪贴板不可用时保持原状
  }
}

/** 标志词本地化；未知词原样显示。 */
function usageLabel(head: string): string {
  const key = USAGE_LABEL_KEY[head.toLowerCase()]
  return key ? t(key) : head
}

function isEnabled(target: SkillTargetState): boolean {
  return target.state === 'linked'
}

/** 选中列表项 → 右侧加载详情（纯本地状态，不跳路由）。 */
async function select(skill: SkillSummary): Promise<void> {
  if (selectedName.value === skill.name && doc.value) return
  selectedName.value = skill.name
  editing.value = false
  await loadDoc(skill.name)
}

async function loadDoc(name: string): Promise<void> {
  loadingDoc.value = true
  try {
    doc.value = await store.read(name)
    draftName.value = doc.value.frontmatter.name ?? name
    draftDescription.value = doc.value.frontmatter.description ?? ''
    draftBody.value = doc.value.body ?? ''
  } catch (e) {
    doc.value = null
    toast.error(t('skills.loadFailed'), {
      message: e instanceof Error ? e.message : String(e),
    })
  } finally {
    loadingDoc.value = false
  }
}

/** 点击工具徽章即切换该工具的启停。 */
async function toggle(skill: SkillSummary, target: SkillTargetState): Promise<void> {
  const key = `${skill.name}:${target.toolId}`
  if (busy.value) return
  busy.value = key
  try {
    const summary = await store.setEnabled(skill.name, target.toolId, !isEnabled(target))
    // 详情面板同步最新状态（列表已由 store 就地更新）。
    if (doc.value?.summary.name === summary.name) doc.value.summary = summary
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e)
    toast.error(t('skills.toggleFailed'), { message, duration: 6000 })
  } finally {
    busy.value = ''
  }
}

async function revealSkill(skill: SkillSummary): Promise<void> {
  try {
    const dir = await store.dirPath(skill.name)
    await openPath(dir)
  } catch (e) {
    toast.error(t('skills.openFailed'), {
      message: e instanceof Error ? e.message : String(e),
    })
  }
}

async function revealTools(): Promise<void> {
  const root = store.sourceRoot
  if (!root) return
  try {
    await openPath(root)
  } catch (e) {
    toast.error(t('skills.openFailed'), {
      message: e instanceof Error ? e.message : String(e),
    })
  }
}

/** 刷新；错误交给全局 toast（避免未捕获 rejection）。 */
async function refresh(): Promise<void> {
  try {
    await store.refresh()
  } catch (e) {
    toast.error(t('skills.loadFailed'), {
      message: e instanceof Error ? e.message : String(e),
    })
  }
}

/** 项目级作用域：选择目录后立即扫描。 */
async function chooseProjectDir(): Promise<void> {
  try {
    const dir = await openDialog({ directory: true, multiple: false })
    if (typeof dir !== 'string') return
    store.setScope('project', dir)
    await refresh()
  } catch (e) {
    toast.error(t('skills.openFailed'), {
      message: e instanceof Error ? e.message : String(e),
    })
  }
}

async function useGlobalScope(): Promise<void> {
  store.setScope('global')
  await refresh()
}

/** 新建：清空选中，右侧面板进入创建态。 */
function openCreate(): void {
  selectedName.value = ''
  doc.value = null
  draftName.value = ''
  draftDescription.value = ''
  draftBody.value = ''
  editing.value = true
}

/** 进入编辑态（表单从当前详情初始化）。 */
function startEdit(): void {
  if (!doc.value) return
  draftName.value = doc.value.frontmatter.name ?? doc.value.summary.name
  draftDescription.value = doc.value.frontmatter.description ?? ''
  draftBody.value = doc.value.body ?? ''
  editing.value = true
}

/** 取消编辑；创建态直接回到未选中。 */
function cancelEdit(): void {
  editing.value = false
  if (!doc.value) selectedName.value = ''
}

/** 保存（新建 / 编辑共用）；写穿真源链接，外部改动由 rawExpected 拦截。 */
async function saveDoc(): Promise<void> {
  const name = draftName.value.trim()
  const creating = !doc.value
  if (creating && !NAME_RE.test(name)) {
    toast.error(t('skills.saveFailed'), { message: t('skills.nameInvalid') })
    return
  }
  if (!draftDescription.value.trim()) {
    toast.error(t('skills.saveFailed'), { message: t('skills.descRequired') })
    return
  }

  saving.value = true
  try {
    const input = {
      name,
      description: draftDescription.value,
      body: draftBody.value,
      extra: doc.value?.frontmatter.extra ?? {},
      rawExpected: doc.value?.raw ?? null,
    }
    if (creating) await store.create(input)
    else await store.save(name, input)
    toast.success(t(creating ? 'skills.created' : 'skills.saved'))
    selectedName.value = name
    editing.value = false
    await loadDoc(name)
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e)
    const code = (e as { code?: string }).code
    if (code === 'CONFLICT') {
      toast.error(t('skills.conflict'), {
        message,
        duration: 8000,
        action: { label: t('skills.reload'), run: () => void loadDoc(name) },
      })
    } else {
      toast.error(t('skills.saveFailed'), { message, duration: 6000 })
    }
  } finally {
    saving.value = false
  }
}

function requestDelete(): void {
  deletingName.value = doc.value?.summary.name ?? ''
  deleteOpen.value = true
}

async function confirmDelete(): Promise<void> {
  const name = deletingName.value
  if (!name) return
  try {
    await store.remove(name)
    deleteOpen.value = false
    if (selectedName.value === name) {
      selectedName.value = ''
      doc.value = null
      editing.value = false
    }
    toast.success(t('skills.deleted'))
  } catch (e) {
    toast.error(t('skills.deleteFailed'), {
      message: e instanceof Error ? e.message : String(e),
    })
  }
}

/** 顶栏拖拽窗口（与仪表板顶栏共用 useWindowDrag）。 */
const topBarEl = ref<HTMLElement | null>(null)
useWindowDrag(topBarEl)

/** 作用域 / 刷新后自动选中第一条，避免右侧一直空着。 */
watch(
  () => store.skills.map((skill) => skill.name).join('\u0000'),
  () => {
    const names = store.skills.map((skill) => skill.name)
    if (selectedName.value && names.includes(selectedName.value)) return
    const first = names[0]
    if (first) void loadSelected(first)
    else {
      selectedName.value = ''
      doc.value = null
      editing.value = false
    }
  },
)

/** 列表重扫后按名字选中（不需要完整 SkillSummary）。 */
async function loadSelected(name: string): Promise<void> {
  selectedName.value = name
  editing.value = false
  await loadDoc(name)
}

onMounted(refresh)
</script>

<template>
  <div class="dash">
    <header ref="topBarEl" class="dash-top">
      <button
        class="top-brand"
        type="button"
        :title="t('app.backHome')"
        @click="router.push('/projects')"
      >
        <span class="top-logo" aria-hidden="true">
          <img :src="logo" alt="" width="18" height="18" />
        </span>
        <span class="top-title">RustFox</span>
        <span class="top-tag">{{ t('app.tagline') }}</span>
      </button>
      <div class="top-right">
        <HeaderUpdateTag />
        <IconButton
          name="settings"
          :size="15"
          :title="t('settings.title')"
          @click="showSettings = true"
        />
      </div>
    </header>

    <div class="dash-body">
      <DashboardNav />

      <main class="skills-main">
        <!-- 左：skill 列表（选中即右侧详情，不发生路由跳转） -->
        <aside class="skills-list-pane">
          <div class="list-head">
            <div class="scope-tabs" role="tablist">
              <button
                type="button"
                class="scope-tab"
                :class="{ active: store.scope === 'global' }"
                @click="useGlobalScope"
              >
                {{ t('skills.scopeGlobal') }}
              </button>
              <button
                type="button"
                class="scope-tab"
                :class="{ active: store.scope === 'project' }"
                @click="chooseProjectDir"
              >
                {{ t('skills.scopeProject') }}
              </button>
            </div>
            <div class="list-actions">
              <IconButton name="plus" :size="14" :title="t('skills.new')" @click="openCreate" />
              <IconButton
                name="folder-open"
                :size="14"
                :title="t('skills.openSource')"
                @click="revealTools"
              />
              <IconButton
                name="refresh"
                :size="14"
                :title="t('skills.refresh')"
                :disabled="store.loading"
                @click="refresh"
              />
            </div>
          </div>

          <p
            class="path-bar"
            :title="store.scope === 'project' ? (store.projectDir ?? '') : store.sourceRoot"
          >
            <code class="path">
              {{ store.scope === 'project' ? (store.projectDir ?? t('skills.pickDir')) : store.sourceRoot }}
            </code>
            <span class="count-badge">{{ t('skills.count', { n: store.skills.length }) }}</span>
          </p>

          <div class="filters">
            <input
              v-model="query"
              class="rf-input filter-search"
              type="search"
              :placeholder="t('skills.filterSearch')"
              :aria-label="t('skills.filterSearch')"
              spellcheck="false"
            />
            <button
              v-if="filterTools.length"
              class="filter-toggle"
              type="button"
              :class="{ active: toolsOpen || toolFilter.length > 0 }"
              :aria-expanded="toolsOpen"
              :title="t('skills.filterTools')"
              @click="toolsOpen = !toolsOpen"
            >
              <Icon name="filter" :size="12" />
              {{ t('skills.filterTools') }}
              <span v-if="toolFilter.length" class="filter-count">{{ toolFilter.length }}</span>
            </button>
          </div>

          <!-- 工具筛选收纳为一行：默认不占纵向空间，展开后横向滚动。 -->
          <div v-if="toolsOpen && filterTools.length" class="filter-tools">
            <button
              class="filter-tool filter-tool-issues"
              type="button"
              :class="{ active: onlyIssues }"
              :aria-pressed="onlyIssues"
              :title="t('skills.filterIssues')"
              @click="onlyIssues = !onlyIssues"
            >
              <Icon name="alert-triangle" :size="11" />
              {{ t('skills.filterIssues') }}
            </button>
            <span class="filter-divider" aria-hidden="true" />
            <button
              v-for="tool in filterTools"
              :key="tool.id"
              class="filter-tool"
              type="button"
              :class="{ active: toolFilter.includes(tool.id) }"
              :aria-pressed="toolFilter.includes(tool.id)"
              :title="tool.root"
              @click="toggleToolFilter(tool.id)"
            >
              {{ tool.displayName }}
            </button>
          </div>

          <p v-if="activeFilterCount || onlyIssues" class="filter-summary">
            <span class="hint">
              {{ t('skills.filterResult', { n: filteredSkills.length, total: store.skills.length }) }}
            </span>
            <span v-if="onlyIssues" class="filter-pill">
              <Icon name="alert-triangle" :size="11" />
              {{ t('skills.filterIssues') }}
              <button
                class="filter-pill-close"
                type="button"
                :aria-label="t('skills.filterClear')"
                @click="onlyIssues = false"
              >
                ×
              </button>
            </span>
            <button class="filter-clear" type="button" @click="clearFilters">
              {{ t('skills.filterClear') }}
            </button>
          </p>

          <!-- 首次扫描骨架：只占位不跳动，避免「空白 → 列表」的突变。 -->
          <ul v-if="store.loading" class="skill-list" aria-busy="true">
            <li v-for="i in 6" :key="`sk-${i}`" class="skill-item skeleton-item">
              <div class="item-top">
                <span class="state-dot skeleton-dot" aria-hidden="true" />
                <span class="skeleton skeleton-name" />
              </div>
              <span class="skeleton skeleton-desc" />
              <div class="tools-cell">
                <span class="skeleton skeleton-chip" />
                <span class="skeleton skeleton-chip" />
              </div>
            </li>
          </ul>

          <div v-else-if="store.error" class="banner banner-error">
            <Icon name="alert-triangle" :size="14" />
            <span>{{ store.error }}</span>
          </div>

          <div v-else-if="!hasSkills" class="empty">
            <Icon name="package" :size="28" />
            <p>{{ t('skills.empty') }}</p>
            <p class="hint">{{ t('skills.emptyHint') }}</p>
          </div>

          <div v-else-if="!filteredSkills.length" class="empty">
            <p>{{ t('skills.filterEmpty') }}</p>
            <button class="rf-btn rf-btn-sm" type="button" @click="clearFilters">
              {{ t('skills.filterClear') }}
            </button>
          </div>

          <ul v-else class="skill-list">
            <li
              v-for="skill in filteredSkills"
              :key="skill.name"
              class="skill-item"
              :class="{ selected: skill.name === selectedName }"
              @click="select(skill)"
            >
              <div class="item-top">
                <span class="state-dot" :class="itemTone(skill)" aria-hidden="true" />
                <code class="item-name">{{ skill.name }}</code>
                <span v-if="skill.warnings.length" class="warn" :title="skill.warnings[0].message">
                  <Icon name="alert-triangle" :size="12" />
                </span>
                <div class="row-actions">
                  <button
                    class="icon-btn"
                    type="button"
                    :title="t('skills.openDir')"
                    @click.stop="revealSkill(skill)"
                  >
                    <Icon name="folder" :size="14" />
                  </button>
                </div>
              </div>
              <p v-if="skill.description" class="item-desc">{{ skill.description }}</p>
              <div class="tools-cell">
                <ToolChip
                  v-for="target in visibleStates(skill)"
                  :key="target.toolId"
                  :target="target"
                  :busy="chipBusy(skill, target)"
                  @toggle="toggle(skill, target)"
                />
                <!-- 未启用工具：低透明胶囊，hover 提亮；+N 打开完整面板。 -->
                <ToolChip
                  v-for="target in hiddenStates(skill)"
                  :key="`dim:${target.toolId}`"
                  class="chip-dim"
                  :target="target"
                  :busy="chipBusy(skill, target)"
                  @toggle="toggle(skill, target)"
                />
                <template v-if="hiddenStates(skill).length > 2">
                  <div class="tools-hidden">
                    <ToolChip
                      v-for="target in hiddenStates(skill)"
                      :key="target.toolId"
                      :target="target"
                      :busy="chipBusy(skill, target)"
                      @toggle="toggle(skill, target)"
                    />
                  </div>
                </template>
              </div>
            </li>
          </ul>

          <section v-if="store.foreign.length" class="foreign">
            <h2>{{ t('skills.foreignTitle') }}</h2>
            <p class="hint">{{ t('skills.foreignHint') }}</p>
            <ul>
              <li v-for="item in store.foreign" :key="`${item.toolId}:${item.name}`">
                <code class="foreign-tag">{{ item.name }}</code>
                <span class="hint">{{ item.toolId }}</span>
              </li>
            </ul>
          </section>

          <footer v-if="warningCount" class="footer hint pad">
            {{ t('skills.warningCount', { n: warningCount }) }}
          </footer>
        </aside>

        <!-- 右：详情 / 编辑器（同一面板内切换，路由不变） -->
        <section class="detail-pane">
          <div v-if="loadingDoc" class="detail-placeholder hint">{{ t('skills.loading') }}</div>

          <div v-else-if="editing" class="detail-form">
            <header class="detail-head">
              <h1>{{ doc ? t('skills.editTitle') : t('skills.createTitle') }}</h1>
              <div class="detail-actions">
                <button class="rf-btn rf-btn-sm" type="button" :disabled="saving" @click="cancelEdit">
                  {{ t('common.cancel') }}
                </button>
                <button
                  v-if="doc"
                  class="rf-btn rf-btn-sm rf-btn-danger"
                  type="button"
                  :disabled="saving"
                  @click="requestDelete"
                >
                  <Icon name="trash" :size="14" />
                  {{ t('skills.delete') }}
                </button>
                <button
                  class="rf-btn rf-btn-sm rf-btn-primary"
                  type="button"
                  :disabled="saving"
                  @click="saveDoc"
                >
                  <Icon name="save" :size="14" />
                  {{ saving ? t('common.saving') : t('common.save') }}
                </button>
              </div>
            </header>

            <label class="field">
              <span class="label">{{ t('skills.nameLabel') }}</span>
              <input
                v-model="draftName"
                class="rf-input"
                type="text"
                :readonly="!!doc"
                :placeholder="t('skills.namePlaceholder')"
                spellcheck="false"
              />
              <span v-if="doc" class="field-hint">{{ t('skills.nameLocked') }}</span>
            </label>

            <label class="field">
              <span class="label">{{ t('skills.descLabel') }}</span>
              <input
                v-model="draftDescription"
                class="rf-input"
                type="text"
                :placeholder="t('skills.descPlaceholder')"
              />
              <span v-if="!draftDescription.trim()" class="field-hint">
                {{ t('skills.descRequired') }}
              </span>
            </label>

            <div class="field body-field">
              <span class="label">{{ t('skills.bodyLabel') }}</span>
              <JsonCodeMirror
                v-model="draftBody"
                language="markdown"
                :placeholder-text="t('skills.bodyPlaceholder')"
                autofocus
              />
            </div>
          </div>

          <article v-else-if="doc" class="detail-view">
            <header class="detail-head">
              <div class="detail-title">
                <h1>{{ doc.summary.name }}</h1>
                <div class="path-box">
                  <code class="path-value" :title="doc.summary.dir">{{ doc.summary.dir }}</code>
                  <button
                    class="path-copy"
                    type="button"
                    :aria-label="t('skills.copyPath')"
                    :title="copiedPath ? t('skills.copiedPath') : t('skills.copyPath')"
                    @click="copyPath(doc.summary.dir)"
                  >
                    <Icon :name="copiedPath ? 'check' : 'copy'" :size="13" />
                  </button>
                </div>
              </div>
              <div class="detail-actions">
                <button class="rf-btn rf-btn-sm" type="button" @click="revealSkill(doc.summary)">
                  <Icon name="folder" :size="14" />
                  {{ t('skills.openDir') }}
                </button>
                <button class="rf-btn rf-btn-sm rf-btn-primary" type="button" @click="startEdit">
                  <Icon name="pencil" :size="14" />
                  {{ t('common.edit') }}
                </button>
              </div>
            </header>

            <p class="detail-desc">{{ doc.frontmatter.description }}</p>

            <section class="detail-block">
              <h2 class="block-title">{{ t('skills.usageTitle') }}</h2>
              <template v-if="usage.length">
                <div v-for="block in usage" :key="block.head" class="usage-block">
                  <p class="usage-head">{{ usageLabel(block.head) }}</p>
                  <p v-for="line in block.lines" :key="line" class="usage-line">{{ line }}</p>
                  <div v-if="block.spans.length" class="usage-spans">
                    <code v-for="span in block.spans" :key="span" class="usage-span">{{ span }}</code>
                  </div>
                </div>
              </template>
              <p v-else class="hint">{{ t('skills.usageEmpty') }}</p>
            </section>

            <section class="detail-block">
              <h2 class="block-title">{{ t('skills.colTools') }}</h2>
              <div class="tools-cell">
                <ToolChip
                  v-for="target in doc.summary.states"
                  :key="target.toolId"
                  class="tool-tag"
                  :class="{ 'tool-idle': target.state !== 'linked' }"
                  :target="target"
                  :busy="chipBusy(doc.summary, target)"
                  @toggle="toggle(doc.summary, target)"
                />
              </div>
            </section>

            <section v-if="doc.summary.warnings.length" class="detail-block">
              <h2 class="block-title">{{ t('skills.warningsTitle') }}</h2>
              <p v-for="warn in doc.summary.warnings" :key="warn.code" class="detail-warn">
                <Icon name="alert-triangle" :size="13" />
                <span>{{ warn.message }}</span>
              </p>
            </section>

            <section v-if="doc.summary.extraEntries.length" class="detail-block">
              <h2 class="block-title">{{ t('skills.extraTitle') }}</h2>
              <div class="extra-tags">
                <code v-for="entry in doc.summary.extraEntries" :key="entry" class="extra-tag">
                  <Icon name="file" :size="12" />
                  {{ entry }}
                </code>
              </div>
            </section>

            <section class="detail-block">
              <h2 class="block-title">{{ t('skills.bodyLabel') }}</h2>
              <div class="body-frame">
                <div class="body-frame-bar">
                  <Icon name="file" :size="13" />
                  <span class="body-frame-name">SKILL.md</span>
                </div>
                <pre class="body-preview">{{ doc.body || t('skills.bodyEmpty') }}</pre>
              </div>
            </section>
          </article>

          <div v-else class="detail-placeholder">
            <Icon name="package" :size="28" />
            <p class="hint">{{ t('skills.pickSkill') }}</p>
          </div>
        </section>
      </main>
    </div>

    <SettingsDialog v-model:open="showSettings" />

    <DeleteConfirmModal
      v-model:open="deleteOpen"
      :title="t('skills.deleteTitle')"
      :message="t('skills.deleteMessage', { name: deletingName })"
      :confirm-label="t('skills.delete')"
      @confirm="confirmDelete"
    />
  </div>
</template>

<style scoped>
/* 与仪表板同构：顶栏 + 左侧导航 + 主区（主区内左右分栏）。 */
.dash {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--bg-app);
}

.dash-top {
  height: 56px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 0 16px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-panel);
}

.top-brand {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 8px;
  border: none;
  border-radius: var(--radius-md);
  background: none;
  color: var(--text-1);
  font: inherit;
  cursor: pointer;
}

.top-brand:hover {
  background: var(--bg-hover);
}

.top-logo {
  display: inline-flex;
}

.top-title {
  font-size: 13px;
  font-weight: 600;
}

.top-tag {
  font-size: 11px;
  color: var(--text-3);
}

.top-right {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 10px;
}

.dash-body {
  flex: 1;
  min-height: 0;
  display: flex;
}

.skills-main {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(260px, 340px) minmax(0, 1fr);
}

/* ---------- 左：列表 ---------- */
.skills-list-pane {
  min-width: 0;
  min-height: 0;
  overflow-y: auto;
  scrollbar-gutter: stable;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px;
  border-right: 1px solid var(--border);
  background: var(--bg-panel);
}

.list-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.list-actions {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 2px;
}

.scope-tabs {
  display: flex;
  flex: 0 0 auto;
  gap: 4px;
  padding: 2px;
  border-radius: var(--radius-md);
  background: var(--bg-2);
}

.scope-tab {
  padding: 4px 10px;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-2);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}

.scope-tab.active {
  background: var(--bg-hover);
  color: var(--text-1);
}

.path-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  min-width: 0;
  margin: 0;
}

.path {
  min-width: 0;
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.count-badge {
  flex: 0 0 auto;
  padding: 2px 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--accent) 18%, transparent);
  color: var(--accent);
  font-family: var(--font-mono);
  font-size: 11px;
  font-weight: 600;
  white-space: nowrap;
}

.pad {
  padding: 0 2px;
}

/* ---------- 筛选 ---------- */
.filters {
  display: flex;
  align-items: center;
  gap: 6px;
}

.filter-search {
  flex: 1;
  min-width: 0;
  height: 28px;
  font-size: 12px;
}

.filter-toggle {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 28px;
  padding: 0 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: none;
  color: var(--text-3);
  font: inherit;
  font-size: 11.5px;
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    border-color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}

.filter-toggle:hover {
  color: var(--text-2);
}

.filter-toggle.active {
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  background: var(--accent-tint);
  color: var(--text-1);
}

.filter-count {
  min-width: 14px;
  padding: 0 4px;
  border-radius: 999px;
  background: var(--accent);
  color: var(--bg-app);
  font-size: 10px;
  font-weight: 700;
  line-height: 14px;
  text-align: center;
}

/* 展开后仍只占一行：横向滚动，不换行、不撑高列表。 */
.filter-tools {
  display: flex;
  flex-wrap: nowrap;
  gap: 4px;
  overflow-x: auto;
  padding-bottom: 2px;
  scrollbar-width: none;
}

.filter-tools::-webkit-scrollbar {
  display: none;
}

.filter-tool {
  flex: 0 0 auto;
  padding: 1px 8px;
  border: 1px solid transparent;
  border-radius: 999px;
  background: var(--bg-2);
  color: var(--text-3);
  font: inherit;
  font-size: 11px;
  line-height: 18px;
  white-space: nowrap;
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    border-color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}

.filter-tool:hover {
  color: var(--text-2);
}

.filter-tool.active {
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  background: var(--accent-tint);
  color: var(--text-1);
}

.filter-tool-issues.active {
  border-color: color-mix(in srgb, var(--warning) 45%, transparent);
  background: var(--warning-tint);
}

.filter-divider {
  flex: 0 0 auto;
  align-self: center;
  width: 1px;
  height: 14px;
  background: var(--border);
}

.filter-summary {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
  margin: 0;
}

.filter-pill {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 4px 1px 7px;
  border: 1px solid color-mix(in srgb, var(--warning) 40%, transparent);
  border-radius: 999px;
  background: var(--warning-tint);
  color: var(--text-1);
  font-size: 11px;
  line-height: 17px;
}

.filter-pill-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 15px;
  height: 15px;
  padding: 0;
  border: none;
  border-radius: 50%;
  background: none;
  color: var(--text-3);
  font: inherit;
  font-size: 12px;
  line-height: 1;
  cursor: pointer;
}

.filter-pill-close:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

.filter-clear {
  padding: 0;
  border: none;
  background: none;
  color: var(--accent);
  font: inherit;
  font-size: 11.5px;
  cursor: pointer;
}

.filter-clear:hover {
  text-decoration: underline;
}

/* skill 条目：卡片式（细边框 + 半透明底），选中态用强调色描边 + 左侧指示条。 */
.skill-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.skill-item {
  position: relative;
  padding: 8px 10px;
  border: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--bg-2) 55%, transparent);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}

.skill-item:hover,
.skill-item:focus-within {
  z-index: 1;
  border-color: color-mix(in srgb, var(--border) 100%, transparent);
  background: var(--bg-hover);
}

.skill-item.selected {
  border-color: color-mix(in srgb, var(--accent) 35%, transparent);
  background: var(--accent-tint);
  box-shadow: var(--shadow-sm);
}

.skill-item.selected::before {
  content: '';
  position: absolute;
  left: 0;
  top: 10px;
  bottom: 10px;
  width: 2px;
  border-radius: 2px;
  background: var(--accent);
}

/* 状态点：全绿=ok、有分叉/失效=warn、全未启用=off。 */
.state-dot {
  flex: 0 0 auto;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--text-3);
  opacity: 0.55;
}

.state-dot.ok {
  background: var(--success);
  opacity: 1;
}

.state-dot.warn {
  background: var(--warning);
  opacity: 1;
}

.item-top {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.item-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-1);
  font-family: var(--font-mono);
  font-size: 12.5px;
  font-weight: 700;
}

.warn {
  color: var(--warning);
  display: inline-flex;
  flex: 0 0 auto;
}

.item-desc {
  margin: 2px 0 0;
  font-size: 11.5px;
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row-actions {
  flex: 0 0 auto;
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 2px;
  opacity: 0;
  transition: opacity var(--dur) var(--ease);
}

.skill-item:hover .row-actions,
.skill-item:focus-within .row-actions {
  opacity: 1;
}

.icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-2);
  cursor: pointer;
}

.icon-btn:hover {
  background: var(--bg-active);
  color: var(--text-1);
}

/* 首次扫描骨架：静态灰块，不做无限动画（prefers-reduced-motion 下也安全）。 */
.skeleton {
  display: block;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--text-3) 16%, transparent);
}

.skeleton-item {
  cursor: default;
  pointer-events: none;
}

.skeleton-dot {
  background: color-mix(in srgb, var(--text-3) 16%, transparent);
  opacity: 1;
}

.skeleton-name {
  width: 42%;
  height: 12px;
}

.skeleton-desc {
  width: 76%;
  height: 10px;
  margin-top: 7px;
}

.skeleton-chip {
  width: 62px;
  height: 18px;
  border-radius: var(--radius-md);
}

/* ---------- 工具徽章容器（ToolChip 自带样式在其组件内） ---------- */
.tools-cell {
  position: relative;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 5px;
  margin-top: 6px;
  min-width: 0;
}

/* 详情页工具标签：未启用的低调显示，hover / 聚焦时提亮（可点即启用）。 */
.tool-tag {
  transition: opacity var(--dur) var(--ease);
}

.tool-tag.tool-idle {
  opacity: 0.45;
}

.tool-tag.tool-idle:hover,
.tool-tag.tool-idle:focus-visible {
  opacity: 1;
}

/* 「+N」常驻计数：宽度固定，悬停/聚焦时才弹出未启用工具层，因此不会引起回流。 */
.tools-more {
  padding: 1px 7px;
  border: 1px dashed color-mix(in srgb, var(--border) 90%, transparent);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-3);
  font: inherit;
  font-size: 11px;
  line-height: 18px;
  cursor: pointer;
}

.tools-more:hover,
.tools-more:focus-visible {
  color: var(--text-2);
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
}

/* 绝对定位的悬浮层：脱离文档流，展开时列表高度、滚动条全都不变。 */
.tools-hidden {
  position: absolute;
  top: 100%;
  left: 0;
  z-index: 8;
  display: none;
  flex-wrap: wrap;
  gap: 5px;
  max-width: 100%;
  padding: 6px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-panel);
  box-shadow: var(--shadow);
}

.tools-cell:hover .tools-hidden,
.tools-cell:focus-within .tools-hidden {
  display: flex;
}

.foreign {
  margin-top: 4px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-2);
}

.foreign h2 {
  margin: 0 0 4px;
  font-size: 12px;
  font-weight: 600;
}

.foreign ul {
  margin: 8px 0 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.foreign li {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.foreign-tag {
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-panel);
  color: var(--text-2);
  font-family: var(--font-mono);
  font-size: 11px;
}

/* ---------- 右：详情 / 编辑器 ---------- */
.detail-pane {
  min-width: 0;
  min-height: 0;
  overflow-y: auto;
  scrollbar-gutter: stable;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 20px 24px 40px;
  background: var(--bg);
}

.detail-view,
.detail-form {
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-width: 0;
}

.detail-head {
  display: flex;
  align-items: flex-start;
  gap: 12px;
}

.detail-head h1 {
  margin: 0;
  font-size: 19px;
  font-weight: 700;
  font-family: var(--font-mono);
  color: var(--text-1);
  letter-spacing: -0.01em;
}

.detail-title {
  min-width: 0;
}

/* 路径代码框：单行省略 + 一键复制。 */
.path-box {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  width: fit-content;
  max-width: 100%;
  margin: 8px 0 2px;
  padding: 5px 6px 5px 11px;
  border: 1px solid color-mix(in srgb, var(--border) 75%, transparent);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--bg-panel) 85%, transparent);
}

.path-value {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-3);
  font-family: var(--font-mono);
  font-size: 11.5px;
}

.path-copy {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-3);
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}

.path-copy:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

.detail-actions {
  margin-left: auto;
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 8px;
}

.detail-desc {
  margin: 0;
  font-size: 13px;
  color: var(--text-2);
  line-height: 1.6;
}

/* ---------- 使用方式（从 description 提取的调用形态） ---------- */
.usage-block {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px 10px;
  border: 1px solid color-mix(in srgb, var(--border) 80%, transparent);
  border-radius: var(--radius-md);
  background: var(--bg-panel);
}

.usage-block + .usage-block {
  margin-top: 6px;
}

.usage-head {
  margin: 0;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-3);
}

.usage-line {
  margin: 0;
  font-size: 12px;
  color: var(--text-2);
  line-height: 1.6;
}

.usage-spans {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 2px;
}

.usage-span {
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-2);
  color: var(--text-1);
  font-family: var(--font-mono);
  font-size: 11.5px;
  cursor: text;
  user-select: all;
}

.detail-block {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.block-title {
  margin: 0;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-3);
}

.detail-warn {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0;
  font-size: 12px;
  color: var(--warning);
}

.extra-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.extra-tag {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 9px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-2);
  color: var(--text-2);
  font-family: var(--font-mono);
  font-size: 11px;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease),
    color var(--dur) var(--ease);
}

.extra-tag:hover {
  border-color: color-mix(in srgb, var(--accent) 35%, transparent);
  background: var(--bg-hover);
  color: var(--text-1);
}

/* 正文容器：顶部工具栏 + 圆角内嵌预览区。 */
.body-frame {
  border: 1px solid color-mix(in srgb, var(--border) 75%, transparent);
  border-radius: var(--radius-lg);
  background: var(--bg-2);
  overflow: hidden;
}

.body-frame-bar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  background: color-mix(in srgb, var(--bg-panel) 60%, transparent);
  color: var(--text-3);
}

.body-frame-name {
  font-family: var(--font-mono);
  font-size: 11px;
  letter-spacing: 0.02em;
}

/* 正文只读预览：等宽、可换行、限高可滚，避免长 skill 撑爆面板。 */
.body-preview {
  margin: 0;
  max-height: 46vh;
  overflow: auto;
  padding: 12px 14px;
  background: var(--bg-app);
  color: var(--text-2);
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 1.7;
  white-space: pre-wrap;
  word-break: break-word;
}

.detail-placeholder {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--text-3);
}

.detail-placeholder p {
  margin: 0;
}

/* ---------- 表单 ---------- */
.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.label {
  font-size: 12px;
  color: var(--text-2);
}

.field-hint {
  font-size: 11px;
  color: var(--text-3);
}

.body-field {
  flex: 1;
  min-height: 260px;
}

.banner {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-radius: var(--radius-md);
  font-size: 12px;
}

.banner-error {
  background: var(--danger-tint);
  color: var(--danger-fg);
}

.empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 32px 0;
  color: var(--text-3);
}

.empty p {
  margin: 0;
  font-size: 13px;
}

.hint {
  font-size: 12px;
  color: var(--text-3);
}
</style>