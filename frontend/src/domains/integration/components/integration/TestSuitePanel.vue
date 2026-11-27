<script setup lang="ts">
/**
 * TestSuitePanel：测试套件管理（列表 + 详情编辑）。
 *
 * - 左侧套件列表（名称 / 成员数 / 归属目录），右侧详情；
 * - 详情：名称 / 说明 / 归属目录 / 成员（启用开关、上移下移、移出）/
 *   从场景库添加成员；保存为整体覆盖写（顺序 + 启用一并落库）；
 * - 操作：保存、运行套件（打开批量配置）、复制、删除；
 * - 脏状态保护：切换套件 / 新建 / 删除前确认，避免未保存修改被静默丢弃。
 *
 * 执行不在本组件：运行按钮把 suite_id 交给 testSuites store，由 Rust 批量
 * 引擎调度，进度与报告统一在 BatchReportPanel 展示。
 */
import { computed, onMounted, ref, watch } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { useIntegrationStore } from '@/domains/integration/stores/integration'
import { useTestSuitesStore } from '@/domains/integration/stores/testSuites'
import { useToast } from '@/composables/useToast'
import { errorText } from '@/utils/errorText'
import EmptyState from '@/components/ui/EmptyState.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import Switch from '@/components/ui/Switch.vue'
import Modal from '@/components/ui/Modal.vue'
import IntegrationNavBar from '@/domains/integration/components/integration/IntegrationNavBar.vue'
import type { TestSuite, TestSuiteItem } from '@/types/miyoApi.d'

const t = useLocaleStore().t
const integration = useIntegrationStore()
const store = useTestSuitesStore()
const toast = useToast()

/** 编辑中的草稿（保存前的本地副本）。 */
interface DraftSuite {
  suite: TestSuite
  items: TestSuiteItem[]
}

const draft = ref<DraftSuite | null>(null)
/** 当前草稿对应的已保存快照（脏判断基准；新建套件为 null）。 */
const savedSnapshot = ref<string | null>(null)
const dirty = ref(false)

const selectedId = ref<string | null>(null)
const saving = ref(false)
const addPick = ref<string | null>(null)

/** 切换套件前的脏确认。 */
const pendingSelectId = ref<string | null>(null)
const leaveOpen = ref(false)

/** 目录选项（扁平：目录名；未分类用空值）。 */
const folderOptions = computed(() => [
  { value: '', label: t('testSuite.uncategorized') },
  ...integration.folders.map((f) => ({ value: f.id, label: f.name })),
])

/** 场景名映射（含已删除场景的兜底）。 */
const scenarioNameById = computed(() => {
  const m = new Map<string, string>()
  for (const s of integration.libraryItems) m.set(s.id, s.name)
  return m
})

function scenarioName(id: string): string {
  return scenarioNameById.value.get(id) ?? t('testSuite.scenarioDeleted')
}

/** 可添加的场景（排除已在套件内的）。 */
const addableOptions = computed(() => {
  const inSuite = new Set((draft.value?.items ?? []).map((i) => i.scenario_id))
  return integration.libraryItems
    .filter((s) => !inSuite.has(s.id))
    .map((s) => ({ value: s.id, label: s.name }))
})

function snapshotOf(suite: TestSuite, items: TestSuiteItem[]): string {
  return JSON.stringify({
    name: suite.name,
    description: suite.description,
    folder_id: suite.folder_id ?? null,
    items: items.map((i) => [i.id, i.scenario_id, i.enabled]),
  })
}

function loadSuite(id: string | null): void {
  selectedId.value = id
  store.setActiveSuite(id)
  const found = store.suites.find((s) => s.id === id)
  if (!found) {
    draft.value = null
    savedSnapshot.value = null
    dirty.value = false
    return
  }
  draft.value = {
    suite: {
      id: found.id,
      project_id: found.project_id,
      name: found.name,
      description: found.description,
      folder_id: found.folder_id ?? null,
      created_at: found.created_at,
      updated_at: found.updated_at,
    },
    items: found.items.map((i) => ({ ...i })),
  }
  savedSnapshot.value = snapshotOf(draft.value.suite, draft.value.items)
  dirty.value = false
  addPick.value = null
}

watch(
  () => draft.value,
  (d) => {
    if (!d) return
    dirty.value =
      savedSnapshot.value !== null && snapshotOf(d.suite, d.items) !== savedSnapshot.value
  },
  { deep: true },
)

/** 首次加载 / 项目切换：拉套件 + 场景库 + 目录，并选中第一个套件。 */
async function reload(): Promise<void> {
  if (!integration.projectId) return
  await Promise.all([
    store.loadSuites(),
    integration.refreshLibrary().catch(() => {}),
  ])
  // 路由 / 外部指定优先（深链 ?suite=id）；否则保留当前选择或选第一个。
  const wanted = store.activeSuiteId ?? selectedId.value
  const target =
    wanted && store.suites.some((s) => s.id === wanted) ? wanted : (store.suites[0]?.id ?? null)
  if (target !== selectedId.value) loadSuite(target)
  else if (target) loadSuite(target)
}

onMounted(() => {
  void reload()
})

watch(
  () => integration.projectId,
  () => {
    selectedId.value = null
    draft.value = null
    void reload()
  },
)

// 路由 / 外部入口选中套件（?suite=id）：无脏改动时跟随。
watch(
  () => store.activeSuiteId,
  (id) => {
    if (id === selectedId.value) return
    if (dirty.value) return
    loadSuite(id)
  },
)

/** 详情层级「返回测试套件」：回到列表（保留窄屏列表可见）。 */
function backToSuiteList(): void {
  if (dirty.value) {
    pendingSelectId.value = '__back__'
    leaveOpen.value = true
    return
  }
  loadSuite(null)
}

function newSuite(): void {
  if (dirty.value) {
    pendingSelectId.value = '__new__'
    leaveOpen.value = true
    return
  }
  createBlank()
}

function createBlank(): void {
  const now = new Date().toISOString()
  const projectId = integration.projectId
  if (!projectId) return
  draft.value = {
    suite: {
      id: crypto.randomUUID(),
      project_id: projectId,
      name: t('testSuite.newSuiteName'),
      description: '',
      folder_id: null,
      created_at: now,
      updated_at: now,
    },
    items: [],
  }
  selectedId.value = draft.value.suite.id
  store.setActiveSuite(draft.value.suite.id)
  savedSnapshot.value = null
  dirty.value = true
  addPick.value = null
}

/** 点击左侧列表（脏状态先确认）。 */
function selectSuite(id: string): void {
  if (id === selectedId.value) return
  if (dirty.value) {
    pendingSelectId.value = id
    leaveOpen.value = true
    return
  }
  loadSuite(id)
}

function discardAndLeave(): void {
  const target = pendingSelectId.value
  leaveOpen.value = false
  pendingSelectId.value = null
  if (target === '__new__') createBlank()
  else if (target === '__back__') loadSuite(null)
  else if (target) loadSuite(target)
}

function cancelLeave(): void {
  leaveOpen.value = false
  pendingSelectId.value = null
}

function addScenario(): void {
  const d = draft.value
  const scenarioId = addPick.value
  if (!d || !scenarioId) return
  d.items.push({
    id: crypto.randomUUID(),
    suite_id: d.suite.id,
    scenario_id: scenarioId,
    sort_order: d.items.length,
    enabled: true,
  })
  addPick.value = null
  dirty.value = true
}

function removeItem(index: number): void {
  const d = draft.value
  if (!d) return
  d.items.splice(index, 1)
  dirty.value = true
}

function moveItem(index: number, delta: number): void {
  const d = draft.value
  if (!d) return
  const next = index + delta
  if (next < 0 || next >= d.items.length) return
  const [item] = d.items.splice(index, 1)
  d.items.splice(next, 0, item)
  dirty.value = true
}

/** 保存并离开（切换套件 / 返回列表 / 新建前确认用）。 */
async function saveAndLeave(): Promise<void> {
  await save()
  if (dirty.value) return
  discardAndLeave()
}

async function save(): Promise<void> {
  const d = draft.value
  if (!d) return
  if (!d.suite.name.trim()) {
    toast.warning(t('testSuite.saveFail'))
    return
  }
  saving.value = true
  try {
    const saved = await store.saveSuite(
      { ...d.suite, updated_at: new Date().toISOString() },
      d.items.map((item, index) => ({ ...item, sort_order: index })),
    )
    selectedId.value = saved.id
    loadSuite(saved.id)
    toast.success(t('testSuite.saved'))
  } catch (err) {
    toast.error(t('testSuite.saveFail'), { message: errorText(err) })
  } finally {
    saving.value = false
  }
}

function runSuite(): void {
  const id = selectedId.value
  if (!id) return
  store.openRunDialog({ suite_id: id })
}

/** 打开历史批次（无在途批次时回看报告）。 */
function openHistory(): void {
  void store.openPanel()
}

async function duplicate(): Promise<void> {
  const id = selectedId.value
  if (!id) return
  if (dirty.value) {
    await save()
    if (dirty.value) return
  }
  try {
    const copy = await store.duplicateSuite(id)
    if (copy) {
      loadSuite(copy.id)
      toast.success(t('testSuite.duplicated', { name: copy.name }))
    }
  } catch (err) {
    toast.error(t('testSuite.saveFail'), { message: errorText(err) })
  }
}

const deleteOpen = ref(false)

function confirmDelete(): void {
  deleteOpen.value = true
}

async function doDelete(): Promise<void> {
  const id = selectedId.value
  if (!id) return
  try {
    await store.removeSuite(id)
    deleteOpen.value = false
    toast.success(t('testSuite.deleted'))
    loadSuite(store.suites[0]?.id ?? null)
  } catch (err) {
    toast.error(t('testSuite.saveFail'), { message: errorText(err) })
  }
}
</script>

<template>
  <section class="tsp-root">
    <!-- 左：套件列表 -->
    <aside class="tsp-list">
      <header class="tsp-list-head">
        <div class="tsp-list-titles">
          <h2 class="tsp-title">{{ t('testSuite.title') }}</h2>
          <p class="tsp-subtitle">{{ t('testSuite.subtitle') }}</p>
        </div>
        <div class="tsp-list-actions">
          <button class="miyo-btn miyo-btn-sm miyo-btn-primary" type="button" @click="newSuite">
            <Icon name="plus" :size="12" />{{ t('testSuite.new') }}
          </button>
          <IconButton
            name="history"
            :size="14"
            :title="t('testSuite.history')"
            @click="openHistory"
          />
        </div>
      </header>

      <EmptyState
        v-if="!store.suites.length && !store.suitesLoading"
        icon="beaker"
        :title="t('testSuite.empty')"
        :description="t('testSuite.emptyHint')"
        compact
      />
      <div v-else class="tsp-list-body">
        <button
          v-for="s in store.suites"
          :key="s.id"
          type="button"
          class="tsp-list-row"
          :class="{ active: s.id === selectedId }"
          @click="selectSuite(s.id)"
        >
          <span class="tsp-list-name" :title="s.name">{{ s.name || t('testSuite.unnamed') }}</span>
          <span class="tsp-list-count">{{ t('testSuite.memberCount', { n: s.items.length }) }}</span>
        </button>
      </div>
    </aside>

    <!-- 右：详情 -->
    <div class="tsp-detail">
      <div v-if="!draft" class="tsp-placeholder">
        <EmptyState icon="beaker" :title="t('testSuite.empty')" :description="t('testSuite.selectHint')" compact />
      </div>

      <template v-else>
        <header class="tsp-detail-head">
          <IntegrationNavBar
            :back-label="t('integration.navBackSuites')"
            :crumbs="[{ label: t('integration.title') }, { label: t('testSuite.title') }]"
            suffix
            @back="backToSuiteList"
          />
          <span class="tsp-nav-slash" aria-hidden="true">/</span>
          <input
            v-model="draft.suite.name"
            class="miyo-input tsp-name"
            :placeholder="t('testSuite.namePh')"
            maxlength="120"
          />
          <span v-if="dirty" class="tsp-dirty">{{ t('testSuite.unsaved') }}</span>
          <span class="tsp-head-tools">
            <button
              class="miyo-btn miyo-btn-sm"
              type="button"
              :disabled="!dirty || saving"
              @click="save"
            >
              <Icon name="save" :size="12" />{{ t('testSuite.save') }}
            </button>
            <button class="miyo-btn miyo-btn-sm miyo-btn-primary" type="button" @click="runSuite">
              <Icon name="play" :size="12" />{{ t('testSuite.run') }}
            </button>
            <IconButton name="copy" :size="13" :title="t('testSuite.duplicate')" @click="duplicate" />
            <IconButton name="trash" :size="13" tone="danger" :title="t('testSuite.delete')" @click="confirmDelete" />
          </span>
        </header>

        <div class="tsp-meta">
          <label class="tsp-field">
            <span class="tsp-field-label">{{ t('testSuite.descriptionPh') }}</span>
            <input v-model="draft.suite.description" class="miyo-input" :placeholder="t('testSuite.descriptionPh')" />
          </label>
          <label class="tsp-field">
            <span class="tsp-field-label">{{ t('testSuite.folder') }}</span>
            <CustomSelect
              :model-value="draft.suite.folder_id ?? ''"
              :options="folderOptions"
              size="sm"
              @update:model-value="
                (v: string | number) => (draft!.suite.folder_id = String(v) || null)
              "
            />
          </label>
        </div>

        <div class="tsp-members">
          <div class="tsp-members-head">
            <span class="tsp-members-title">{{ t('testSuite.memberCount', { n: draft.items.length }) }}</span>
            <span class="tsp-add">
              <CustomSelect
                v-model="addPick"
                :options="addableOptions"
                :placeholder="addableOptions.length ? t('testSuite.addScenarioPh') : t('testSuite.allAdded')"
                size="sm"
                :disabled="!addableOptions.length"
              />
              <button
                class="miyo-btn miyo-btn-sm"
                type="button"
                :disabled="!addPick"
                @click="addScenario"
              >
                <Icon name="plus" :size="12" />{{ t('testSuite.addScenario') }}
              </button>
            </span>
          </div>

          <p v-if="!draft.items.length" class="tsp-members-empty">{{ t('testSuite.noMembers') }}</p>
          <div v-for="(item, index) in draft.items" :key="item.id" class="tsp-member">
            <Switch
              :model-value="item.enabled"
              size="sm"
              :label="t('testSuite.enableItem')"
              @update:model-value="(v: boolean) => { item.enabled = v; dirty = true }"
            />
            <span class="tsp-member-name" :class="{ disabled: !item.enabled }" :title="scenarioName(item.scenario_id)">
              {{ scenarioName(item.scenario_id) }}
            </span>
            <span class="tsp-member-tools">
              <IconButton
                name="chevron-up"
                :size="12"
                :title="t('testSuite.moveUp')"
                :disabled="index === 0"
                @click="moveItem(index, -1)"
              />
              <IconButton
                name="chevron-down"
                :size="12"
                :title="t('testSuite.moveDown')"
                :disabled="index === draft.items.length - 1"
                @click="moveItem(index, 1)"
              />
              <IconButton name="x" :size="12" :title="t('testSuite.removeMember')" @click="removeItem(index)" />
            </span>
          </div>
        </div>
      </template>
    </div>

    <!-- 脏状态切换确认 -->
    <Modal
      :open="leaveOpen"
      :title="t('testSuite.unsaved')"
      width="400px"
      @update:open="(v: boolean) => !v && cancelLeave()"
    >
      <p class="tsp-confirm-body">{{ t('testSuite.unsavedBody') }}</p>
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="cancelLeave">
          {{ t('testSuite.cancelAction') }}
        </button>
        <button class="miyo-btn miyo-btn-sm miyo-btn-danger-solid" type="button" @click="discardAndLeave">
          {{ t('testSuite.discard') }}
        </button>
        <button class="miyo-btn miyo-btn-sm miyo-btn-primary" type="button" :disabled="saving" @click="saveAndLeave">
          {{ t('testSuite.saveAndLeave') }}
        </button>
      </template>
    </Modal>

    <!-- 删除确认 -->
    <Modal
      :open="deleteOpen"
      :title="t('testSuite.deleteConfirmTitle')"
      width="420px"
      @update:open="(v: boolean) => !v && (deleteOpen = false)"
    >
      <p class="tsp-confirm-body">
        {{ t('testSuite.deleteConfirmBody', { name: draft?.suite.name ?? '' }) }}
      </p>
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="deleteOpen = false">
          {{ t('testSuite.cancelAction') }}
        </button>
        <button class="miyo-btn miyo-btn-sm miyo-btn-danger-solid" type="button" @click="doDelete">
          {{ t('testSuite.delete') }}
        </button>
      </template>
    </Modal>
  </section>
</template>

<style scoped>
.tsp-root {
  display: grid;
  grid-template-columns: 260px minmax(0, 1fr);
  gap: 18px;
  height: 100%;
  min-height: 0;
  padding: 18px;
}

.tsp-list {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-height: 0;
  padding-right: 14px;
  border-right: 1px solid var(--border);
}

.tsp-list-head {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.tsp-title {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-1);
}

.tsp-subtitle {
  margin: 2px 0 0;
  font-size: var(--fs-xs);
  line-height: 1.5;
  color: var(--text-3);
}

.tsp-list-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}

.tsp-list-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  overflow-y: auto;
  min-height: 0;
}

.tsp-list-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 8px 10px;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  background: transparent;
  text-align: left;
  cursor: pointer;
}

.tsp-list-row:hover {
  background: var(--bg-hover);
}

.tsp-list-row.active {
  border-color: var(--border);
  background: var(--bg-selected);
}

.tsp-list-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-sm);
  color: var(--text-1);
}

.tsp-list-count {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.tsp-detail {
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-width: 0;
  min-height: 0;
  overflow-y: auto;
}

.tsp-placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
}

.tsp-detail-head {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.tsp-nav-slash {
  flex: none;
  color: var(--text-3);
  font-size: var(--fs-xxs);
}

.tsp-name {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-md);
  font-weight: 600;
}

.tsp-dirty {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--warning);
}

.tsp-head-tools {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: none;
}

.tsp-meta {
  display: grid;
  grid-template-columns: minmax(0, 2fr) minmax(0, 1fr);
  gap: 12px;
}

.tsp-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.tsp-field-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.tsp-members {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-height: 0;
}

.tsp-members-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.tsp-members-title {
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.tsp-add {
  display: flex;
  align-items: center;
  gap: 8px;
}

.tsp-members-empty {
  margin: 0;
  padding: 18px 0;
  text-align: center;
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.tsp-member {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-card);
}

.tsp-member-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-sm);
  color: var(--text-1);
}

.tsp-member-name.disabled {
  color: var(--text-3);
  text-decoration: line-through;
}

.tsp-member-tools {
  display: flex;
  gap: 2px;
  flex: none;
}

.tsp-confirm-body {
  margin: 0;
  font-size: var(--fs-sm);
  line-height: 1.6;
  color: var(--text-2);
}
</style>