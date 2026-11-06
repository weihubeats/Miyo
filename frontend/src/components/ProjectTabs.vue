<script setup lang="ts">
/**
 * ProjectTabs：顶栏多项目标签条（工作区 / 项目首页共用）。
 * - 点击标签：切换项目并进入工作区（首页点击同样跳转）；
 * - × 关闭标签（快照丢弃；关闭当前项目时 store 自动切相邻标签）；
 * - ⋯ 菜单按三组固定分隔：未打开的项目（点击打开并切换）｜新建项目 + 项目列表｜
 *   当前项目的重命名（短标签）/ 删除（工作区顶栏用，替代原侧栏头部 ⋯）。
 */
import { ref, withDefaults } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useWorkspaceStore } from '../stores/workspace'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import Icon from './ui/Icon.vue'
import Menu, { type MenuItem } from './ui/Menu.vue'

const props = withDefaults(defineProps<{ projectActions?: boolean }>(), { projectActions: false })

const emit = defineEmits<{
  'new-project': []
  'rename-project': []
  'delete-project': []
}>()

const store = useWorkspaceStore()
const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t
const router = useRouter()
const route = useRoute()

const menuRef = ref<InstanceType<typeof Menu> | null>(null)
const moreBtn = ref<HTMLButtonElement | null>(null)

async function openMore(): Promise<void> {
  if (!moreBtn.value) return
  let items: MenuItem[] = []
  try {
    const openIds = new Set(store.openProjects.map((t) => t.id))
    items = (await api.getProjects())
      .filter((p) => !openIds.has(p.id))
      .map((p) => ({
        key: `switch-project:${p.id}`,
        label: p.name,
        icon: 'folder' as const,
      }))
  } catch {
    toast.error(t('projectTabs.loadFail'))
  }
  // 第一组：切换到未打开的项目（无则整组省略）；第二组固定组前分隔
  items.push(
    { key: 'new-project', label: t('projectTabs.newProject'), icon: 'plus', iconAccent: true, dividerBefore: items.length > 0 },
    { key: 'go-projects', label: t('projectTabs.projectList'), icon: 'layout-grid' },
  )
  // 第三组（工作区模式）：当前项目管理——短标签重命名 + 红色删除（二次确认）
  if (props.projectActions && store.project) {
    const name = store.project.name
    items.push(
      { key: 'rename-project', label: t('projectTabs.rename'), icon: 'pencil', dividerBefore: true },
      { key: 'delete-project', label: t('projectTabs.delete'), icon: 'trash', danger: true, confirm: t('projectTabs.deleteConfirm', { name }) },
    )
  }
  menuRef.value?.openAt(moreBtn.value, items, 'left')
}

function onSelect(item: MenuItem): void {
  if (item.key.startsWith('switch-project:')) void switchTo(item.key.slice('switch-project:'.length))
  else if (item.key === 'new-project') emit('new-project')
  else if (item.key === 'go-projects') router.push('/projects')
  else if (item.key === 'rename-project') emit('rename-project')
}

async function switchTo(projectId: string): Promise<void> {
  try {
    await store.switchProject(projectId)
    // 标签高亮 + 内容切换已是充分反馈，成功不弹 toast（避免频繁切换被打扰）
    if (route.path !== '/workspace') router.push('/workspace')
  } catch (err) {
    toast.error(t('projectTabs.switchFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

function onTabClick(projectId: string): void {
  void switchTo(projectId)
}

function onClose(projectId: string): void {
  store.closeProjectTab(projectId)
}
</script>

<template>
  <div class="ptabs">
    <div class="proj-tabs">
      <div
        v-for="tab in store.openProjects"
        :key="tab.id"
        class="proj-tab"
        :class="{ active: tab.id === store.project?.id }"
        :title="tab.name"
        @click="onTabClick(tab.id)"
      >
        <span class="pt-name">{{ tab.name }}</span>
        <button
          class="pt-close"
          type="button"
          :title="t('projectTabs.closeTab', { name: tab.name })"
          :aria-label="t('projectTabs.closeTab', { name: tab.name })"
          @click.stop="onClose(tab.id)"
        >
          <Icon name="x" :size="11" />
        </button>
      </div>
    </div>
    <button
      ref="moreBtn"
      class="proj-more"
      type="button"
      :title="t('projectTabs.more')"
      :aria-label="t('projectTabs.more')"
      @click="openMore"
    >
      <Icon name="more-horizontal" :size="14" />
    </button>
  </div>
  <Menu ref="menuRef" @select="onSelect" @confirm="emit('delete-project')" />
</template>

<style scoped>
.ptabs {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 1 1 auto;
}

.proj-tabs {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  overflow-x: auto;
  scrollbar-width: none;
}
.proj-tabs::-webkit-scrollbar {
  display: none;
}
/* 悬停时露出细滚动条，暗示可横滚（默认隐藏保简洁） */
.proj-tabs:hover {
  scrollbar-width: thin;
}
.proj-tabs:hover::-webkit-scrollbar {
  display: block;
  height: 6px;
}

.proj-tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 180px;
  padding: 3px 8px 3px 12px;
  border: 1px solid transparent;
  border-radius: var(--radius);
  font-size: 12.5px;
  color: var(--text-2);
  cursor: pointer;
  user-select: none;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}
.proj-tab:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
/* 当前项目：accent 淡底 + 描边 + 左侧圆点，与其他已打开标签明确区分 */
.proj-tab.active {
  background: color-mix(in srgb, var(--accent) 12%, var(--bg-elevated));
  border-color: color-mix(in srgb, var(--accent) 35%, transparent);
  color: var(--text-1);
  font-weight: 600;
}
.proj-tab.active::before {
  content: '';
  width: 6px;
  height: 6px;
  flex-shrink: 0;
  border-radius: 50%;
  background: var(--accent);
  box-shadow: 0 0 4px var(--accent);
}

.pt-name {
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.pt-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  padding: 0;
  border: none;
  border-radius: 4px;
  background: none;
  color: var(--text-3);
  cursor: pointer;
  flex-shrink: 0;
}
.pt-close:hover {
  background: var(--bg-active);
  color: var(--text-1);
}
.proj-tab:not(:hover) .pt-close {
  opacity: 0;
}
.proj-tab.active .pt-close {
  opacity: 1;
}

.proj-more {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: none;
  border-radius: var(--radius);
  background: none;
  color: var(--text-2);
  cursor: pointer;
  flex-shrink: 0;
}
.proj-more:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
</style>
