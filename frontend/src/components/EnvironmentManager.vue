<script setup lang="ts">
/**
 * EnvironmentManager：环境管理工作区（弹窗壳）。
 *
 * 环境管理的唯一编辑实现在 `env/EnvWorkspace.vue`（左右双栏：全局配置 / 环境列表 +
 * 环境详情）。本组件只提供 Modal 外壳——顶栏环境选择器、地址栏「管理环境…」、
 * 速览「管理环境」共用这一个弹窗入口；设置 → 环境页签则直接内嵌 EnvWorkspace
 * （不走本壳），两层复用同一套实现，不存在重复的环境编辑界面。
 *
 * guardClose 转发工作区的三选确认（保存并继续 / 放弃修改 / 继续编辑）；
 * Modal 关闭即卸载内容，工作区每次打开重新挂载并重新加载。
 */
import { computed, ref } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'
import Modal from './ui/Modal.vue'
import EnvWorkspace from './env/EnvWorkspace.vue'

const props = defineProps<{
  open: boolean
  initialEnvId?: string | null
  createNew?: boolean
  /** 独立管理模式：管理指定项目（非当前工作区项目）的环境——设置页跨项目进入。 */
  projectId?: string | null
  projectName?: string | null
}>()

const emit = defineEmits<{ 'update:open': [open: boolean] }>()

const store = useWorkspaceStore()
const locale = useLocaleStore()
const t = locale.t

const workspace = ref<InstanceType<typeof EnvWorkspace> | null>(null)

/** 独立管理模式：目标项目不是当前工作区项目。 */
const detached = computed(() => !!props.projectId && props.projectId !== store.project?.id)

/** 弹窗标题：独立模式追加项目名，明确当前管理的是谁的环境。 */
const dialogTitle = computed(() =>
  detached.value && props.projectName
    ? `${t('settings.environments')} · ${props.projectName}`
    : t('settings.environments'),
)

/** 关闭守卫：转发工作区未保存确认（工作区在 Modal 关闭态下未挂载时直接放行）。 */
function onGuardClose(): boolean {
  return workspace.value?.guardClose() ?? true
}
</script>

<template>
  <Modal
    :open="open"
    :title="dialogTitle"
    width="min(1120px, 94vw)"
    :guard-close="onGuardClose"
    @update:open="emit('update:open', $event)"
  >
    <EnvWorkspace
      ref="workspace"
      :project-id="projectId"
      :project-name="projectName"
      :initial-env-id="initialEnvId"
      :create-new="createNew"
      @close="emit('update:open', false)"
    />
  </Modal>
</template>
